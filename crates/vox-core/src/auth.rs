use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::extract::{Request, State};
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, params};
use subtle::ConstantTimeEq;
use thiserror::Error;
use vox_api::application::{AuthenticatedActor, EstablishedSession, SessionAuthentication};
use vox_api::contract::auth::CreateSessionRequest;
use vox_api::error::{ApiError, ErrorCategory};
use vox_connections::{
    AuditRecord, ConnectionRepository, Permission, Role, RoleId, SqliteConnectionRepository, User,
    UserId,
};

const SESSION_COOKIE: &str = "vox_session";
const CSRF_HEADER: &str = "x-vox-csrf";

pub struct SessionBootstrap {
    pub user_id: UserId,
    pub display_name: String,
    pub permissions: BTreeSet<Permission>,
    pub bootstrap_credential: vox_connections::SecretBytes,
    pub expires_at_unix_ms: i64,
    pub cookie_secure: bool,
}

#[derive(Clone)]
pub struct AuthState {
    sessions: Arc<SessionStore>,
    users: SqliteConnectionRepository,
    manual_execution_permission: Permission,
    bootstrap_hash: String,
    bootstrap_user_id: UserId,
    bootstrap_expires_at_unix_ms: i64,
    cookie_secure: bool,
}

impl AuthState {
    pub fn open(
        database_path: impl AsRef<Path>,
        users: SqliteConnectionRepository,
        bootstrap: SessionBootstrap,
        now_unix_ms: i64,
        manual_execution_permission: Permission,
    ) -> Result<Self, AuthError> {
        if bootstrap.expires_at_unix_ms <= now_unix_ms {
            return Err(AuthError::ExpiredBootstrapSession);
        }
        validate_token(bootstrap.bootstrap_credential.expose_secret())?;
        let bootstrap_hash = digest_hex(bootstrap.bootstrap_credential.expose_secret());
        let user = User {
            id: bootstrap.user_id.clone(),
            display_name: bootstrap.display_name,
            enabled: true,
        };
        let role = Role {
            id: deterministic_bootstrap_role_id()?,
            name: "SERVER_BOOTSTRAP".to_owned(),
            permissions: bootstrap.permissions,
        };
        let correlation_id = format!("bootstrap-provisioning-{}", uuid::Uuid::new_v4());
        let (provisioning_audits, adoption_audit) =
            bootstrap_provisioning_audits(&user, &role, &correlation_id, now_unix_ms);
        users.provision_bootstrap_identity(&user, &role, &provisioning_audits, &adoption_audit)?;

        let sessions = SessionStore::open(database_path)?;
        Ok(Self {
            sessions: Arc::new(sessions),
            users,
            manual_execution_permission,
            bootstrap_hash,
            bootstrap_user_id: user.id,
            bootstrap_expires_at_unix_ms: bootstrap.expires_at_unix_ms,
            cookie_secure: bootstrap.cookie_secure,
        })
    }

    fn authenticate(&self, request: &Request) -> Result<VerifiedSession, AuthFailure> {
        let token = cookie_value(request, SESSION_COOKIE).ok_or(AuthFailure::MissingSession)?;
        let session = self
            .sessions
            .resolve(
                token.as_bytes(),
                now_unix_ms().map_err(|_| AuthFailure::Unavailable)?,
            )?
            .ok_or(AuthFailure::InvalidSession)?;
        let user = self
            .users
            .user(&session.user_id)
            .map_err(|_| AuthFailure::Unavailable)?
            .filter(|user| user.enabled)
            .ok_or(AuthFailure::RevokedUser)?;
        if requires_csrf(request.method()) {
            let supplied = request
                .headers()
                .get(CSRF_HEADER)
                .map(|value| value.as_bytes())
                .ok_or(AuthFailure::MissingCsrf)?;
            let supplied_hash = digest_hex(supplied);
            if !bool::from(supplied_hash.as_bytes().ct_eq(session.csrf_hash.as_bytes())) {
                return Err(AuthFailure::InvalidCsrf);
            }
        }
        let permissions = self
            .users
            .permissions(&user.id)
            .map_err(|_| AuthFailure::Unavailable)?;
        Ok(VerifiedSession {
            actor: AuthenticatedActor {
                user_id: user.id.as_str().to_owned(),
            },
            permissions,
        })
    }
}

#[async_trait]
impl SessionAuthentication for AuthState {
    async fn establish_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<EstablishedSession, ApiError> {
        let now = now_unix_ms().map_err(|_| auth_unavailable())?;
        let supplied_hash = digest_hex(request.bootstrap_credential.as_bytes());
        if now >= self.bootstrap_expires_at_unix_ms
            || !bool::from(
                supplied_hash
                    .as_bytes()
                    .ct_eq(self.bootstrap_hash.as_bytes()),
            )
        {
            return Err(ApiError::new(
                ErrorCategory::Authentication,
                "BOOTSTRAP_CREDENTIAL_REJECTED",
                "trusted bootstrap credential rejected",
            ));
        }
        let user = self
            .users
            .user(&self.bootstrap_user_id)
            .map_err(|_| auth_unavailable())?
            .filter(|user| user.enabled)
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCategory::Authentication,
                    "BOOTSTRAP_PRINCIPAL_REVOKED",
                    "bootstrap principal is disabled or revoked",
                )
            })?;
        let session_token = random_token().map_err(|_| auth_unavailable())?;
        let csrf_token = random_token().map_err(|_| auth_unavailable())?;
        self.sessions
            .upsert(
                &user.id,
                session_token.as_bytes(),
                csrf_token.as_bytes(),
                self.bootstrap_expires_at_unix_ms,
                now,
            )
            .map_err(|_| auth_unavailable())?;
        Ok(EstablishedSession {
            session_token,
            csrf_token,
            expires_at_unix_ms: self.bootstrap_expires_at_unix_ms,
            cookie_secure: self.cookie_secure,
        })
    }
}

struct VerifiedSession {
    actor: AuthenticatedActor,
    permissions: BTreeSet<Permission>,
}

pub async fn authenticated_actor_middleware(
    State(auth): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.uri().path() == "/api/v1/system/health"
        || (*request.method() == Method::POST && request.uri().path() == "/api/v1/auth/session")
    {
        return next.run(request).await;
    }
    match auth.authenticate(&request) {
        Ok(session) => {
            if let Some(required) = required_permission(
                request.method(),
                request.uri().path(),
                auth.manual_execution_permission,
            ) && !session.permissions.contains(&required)
            {
                return auth_error(
                    ErrorCategory::Permission,
                    "RBAC_PERMISSION_DENIED",
                    "authenticated principal lacks required permission",
                );
            }
            request.extensions_mut().insert(session.actor);
            next.run(request).await
        }
        Err(
            AuthFailure::MissingSession | AuthFailure::InvalidSession | AuthFailure::RevokedUser,
        ) => auth_error(
            ErrorCategory::Authentication,
            "AUTHENTICATION_REQUIRED",
            "valid server-side session required",
        ),
        Err(AuthFailure::MissingCsrf | AuthFailure::InvalidCsrf) => auth_error(
            ErrorCategory::Permission,
            "CSRF_VALIDATION_FAILED",
            "state-changing request requires valid CSRF token",
        ),
        Err(AuthFailure::Unavailable) => auth_error(
            ErrorCategory::Transient,
            "AUTHENTICATION_UNAVAILABLE",
            "authentication subsystem unavailable",
        ),
    }
}

fn auth_error(category: ErrorCategory, code: &'static str, message: &'static str) -> Response {
    ApiError::new(category, code, message).into_response()
}

fn bootstrap_provisioning_audits(
    user: &User,
    role: &Role,
    correlation_id: &str,
    occurred_at_unix_ms: i64,
) -> ([AuditRecord; 3], AuditRecord) {
    (
        [
            AuditRecord {
                actor: user.id.clone(),
                action: "BOOTSTRAP_USER_PROVISIONED".to_owned(),
                target_ref: user.id.as_str().to_owned(),
                previous_state: None,
                new_state: Some("ENABLED".to_owned()),
                correlation_id: correlation_id.to_owned(),
                occurred_at_unix_ms,
            },
            AuditRecord {
                actor: user.id.clone(),
                action: "BOOTSTRAP_ROLE_PROVISIONED".to_owned(),
                target_ref: role.id.as_str().to_owned(),
                previous_state: None,
                new_state: Some("SERVER_BOOTSTRAP".to_owned()),
                correlation_id: correlation_id.to_owned(),
                occurred_at_unix_ms,
            },
            AuditRecord {
                actor: user.id.clone(),
                action: "BOOTSTRAP_ROLE_ASSIGNED".to_owned(),
                target_ref: user.id.as_str().to_owned(),
                previous_state: None,
                new_state: Some(role.id.as_str().to_owned()),
                correlation_id: correlation_id.to_owned(),
                occurred_at_unix_ms,
            },
        ],
        AuditRecord {
            actor: user.id.clone(),
            action: "BOOTSTRAP_SECURITY_STATE_ADOPTED".to_owned(),
            target_ref: user.id.as_str().to_owned(),
            previous_state: Some("PERSISTED".to_owned()),
            new_state: Some("AUTHORITATIVE".to_owned()),
            correlation_id: correlation_id.to_owned(),
            occurred_at_unix_ms,
        },
    )
}

fn cookie_value(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(candidate, value)| (candidate == name).then(|| value.to_owned()))
}

fn requires_csrf(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn required_permission(
    method: &Method,
    path: &str,
    manual_execution_permission: Permission,
) -> Option<Permission> {
    if *method == Method::POST && path.starts_with("/api/v1/commands/") {
        return Some(manual_execution_permission);
    }
    if matches!(
        path,
        "/api/v1/runtime"
            | "/api/v1/runtime/scopes"
            | "/api/v1/reconciliation"
            | "/api/v1/accounts"
            | "/api/v1/portfolio"
            | "/api/v1/positions"
            | "/api/v1/orders"
            | "/api/v1/stop-orders"
            | "/api/v1/operations"
            | "/api/v1/mutations"
            | "/api/v1/stream"
    ) {
        return Some(Permission::ViewPortfolio);
    }
    None
}

fn validate_token(token: &[u8]) -> Result<(), AuthError> {
    if !(32..=4096).contains(&token.len()) {
        return Err(AuthError::InvalidTokenLength);
    }
    Ok(())
}

fn deterministic_bootstrap_role_id() -> Result<RoleId, AuthError> {
    RoleId::parse("role:00000000-0000-4000-8000-000000000017")
        .map_err(|_| AuthError::StaticIdentity)
}

#[derive(Clone)]
struct SessionStore {
    path: PathBuf,
}

struct SessionRecord {
    user_id: UserId,
    csrf_hash: String,
}

impl SessionStore {
    fn open(path: impl AsRef<Path>) -> Result<Self, AuthError> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        store.connection()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS auth_sessions (
                session_hash TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES connection_users(user_id) ON DELETE CASCADE,
                csrf_hash TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                expires_at_unix_ms INTEGER NOT NULL,
                revoked_at_unix_ms INTEGER
            ) STRICT;",
        )?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, AuthError> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }

    fn upsert(
        &self,
        user_id: &UserId,
        token: &[u8],
        csrf: &[u8],
        expires_at_unix_ms: i64,
        now_unix_ms: i64,
    ) -> Result<(), AuthError> {
        self.connection()?.execute(
            "INSERT INTO auth_sessions (
                session_hash, user_id, csrf_hash, created_at_unix_ms, expires_at_unix_ms,
                revoked_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL)
             ON CONFLICT(session_hash) DO UPDATE SET
                user_id = excluded.user_id,
                csrf_hash = excluded.csrf_hash,
                expires_at_unix_ms = excluded.expires_at_unix_ms,
                revoked_at_unix_ms = NULL",
            params![
                digest_hex(token),
                user_id.as_str(),
                digest_hex(csrf),
                now_unix_ms,
                expires_at_unix_ms,
            ],
        )?;
        Ok(())
    }

    fn resolve(
        &self,
        token: &[u8],
        now_unix_ms: i64,
    ) -> Result<Option<SessionRecord>, AuthFailure> {
        self.connection()
            .map_err(|_| AuthFailure::Unavailable)?
            .query_row(
                "SELECT user_id, csrf_hash FROM auth_sessions
                 WHERE session_hash = ?1 AND revoked_at_unix_ms IS NULL
                   AND expires_at_unix_ms > ?2",
                params![digest_hex(token), now_unix_ms],
                |row| {
                    Ok(SessionRecord {
                        user_id: UserId::parse(row.get::<_, String>(0)?).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                        csrf_hash: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(|_| AuthFailure::Unavailable)
    }
}

fn digest_hex(value: &[u8]) -> String {
    digest::digest(&digest::SHA256, value)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn random_token() -> Result<String, ring::error::Unspecified> {
    let mut bytes = [0_u8; 32];
    SystemRandom::new().fill(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn auth_unavailable() -> ApiError {
    ApiError::new(
        ErrorCategory::Transient,
        "AUTHENTICATION_UNAVAILABLE",
        "authentication subsystem unavailable",
    )
}

fn now_unix_ms() -> Result<i64, AuthError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| AuthError::Clock)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthFailure {
    MissingSession,
    InvalidSession,
    RevokedUser,
    MissingCsrf,
    InvalidCsrf,
    Unavailable,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("session and CSRF tokens must contain 32 to 4096 bytes")]
    InvalidTokenLength,
    #[error("bootstrap session expiry must be in the future")]
    ExpiredBootstrapSession,
    #[error("system clock unavailable")]
    Clock,
    #[error("compiled bootstrap role identity is invalid")]
    StaticIdentity,
    #[error("authentication persistence failed")]
    Persistence(#[from] rusqlite::Error),
    #[error("connection identity persistence failed")]
    Repository(#[from] vox_connections::RepositoryError),
}
