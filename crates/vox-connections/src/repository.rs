use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::model::{
    AuditRecord, BindingId, BrokerAccount, BrokerAccountBinding, BrokerConnection, ConnectionId,
    ExecutionAuthorization, Permission, ProviderId, Role, RoleId, User, UserId,
};

pub trait ConnectionRepository: Send + Sync {
    fn put_user(&self, user: &User) -> Result<(), RepositoryError>;
    fn put_role(&self, role: &Role) -> Result<(), RepositoryError>;
    fn grant_role(&self, user_id: &UserId, role_id: &RoleId) -> Result<(), RepositoryError>;
    fn permissions(&self, user_id: &UserId) -> Result<BTreeSet<Permission>, RepositoryError>;
    fn insert_connection(&self, connection: &BrokerConnection) -> Result<(), RepositoryError>;
    fn update_connection(&self, connection: &BrokerConnection) -> Result<(), RepositoryError>;
    fn connection(&self, id: &ConnectionId) -> Result<Option<BrokerConnection>, RepositoryError>;
    fn list_connections(&self) -> Result<Vec<BrokerConnection>, RepositoryError>;
    fn replace_accounts(
        &self,
        connection_id: &ConnectionId,
        accounts: &[BrokerAccount],
    ) -> Result<(), RepositoryError>;
    fn accounts(&self, connection_id: &ConnectionId)
    -> Result<Vec<BrokerAccount>, RepositoryError>;
    fn put_binding(&self, binding: &BrokerAccountBinding) -> Result<(), RepositoryError>;
    fn set_binding_enabled(
        &self,
        binding_id: &BindingId,
        enabled: bool,
        now_unix_ms: i64,
    ) -> Result<(), RepositoryError>;
    fn delete_binding(&self, binding_id: &BindingId) -> Result<(), RepositoryError>;
    fn bindings(
        &self,
        connection_id: &ConnectionId,
    ) -> Result<Vec<BrokerAccountBinding>, RepositoryError>;
    fn authorization(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<Option<ExecutionAuthorization>, RepositoryError>;
    fn put_authorization(
        &self,
        authorization: &ExecutionAuthorization,
    ) -> Result<(), RepositoryError>;
    fn append_audit(&self, record: &AuditRecord) -> Result<(), RepositoryError>;
    fn audit_records(&self) -> Result<Vec<AuditRecord>, RepositoryError>;
    fn delete_connection(&self, id: &ConnectionId) -> Result<(), RepositoryError>;
}

#[derive(Clone)]
pub struct SqliteConnectionRepository {
    path: PathBuf,
}

impl SqliteConnectionRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let repository = Self {
            path: path.as_ref().to_path_buf(),
        };
        repository.connection()?.execute_batch(SCHEMA)?;
        Ok(repository)
    }

    fn connection(&self) -> Result<Connection, RepositoryError> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }
}

impl ConnectionRepository for SqliteConnectionRepository {
    fn put_user(&self, user: &User) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "INSERT INTO connection_users (user_id, display_name, enabled) VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET display_name = excluded.display_name,
             enabled = excluded.enabled",
            params![user.id.as_str(), user.display_name, user.enabled],
        )?;
        Ok(())
    }

    fn put_role(&self, role: &Role) -> Result<(), RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO connection_roles (role_id, name) VALUES (?1, ?2)
             ON CONFLICT(role_id) DO UPDATE SET name = excluded.name",
            params![role.id.as_str(), role.name],
        )?;
        transaction.execute(
            "DELETE FROM connection_role_permissions WHERE role_id = ?1",
            [role.id.as_str()],
        )?;
        for permission in &role.permissions {
            transaction.execute(
                "INSERT INTO connection_role_permissions (role_id, permission) VALUES (?1, ?2)",
                params![role.id.as_str(), encode(permission)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn grant_role(&self, user_id: &UserId, role_id: &RoleId) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "INSERT OR IGNORE INTO connection_user_roles (user_id, role_id) VALUES (?1, ?2)",
            params![user_id.as_str(), role_id.as_str()],
        )?;
        Ok(())
    }

    fn permissions(&self, user_id: &UserId) -> Result<BTreeSet<Permission>, RepositoryError> {
        let connection = self.connection()?;
        let enabled = connection
            .query_row(
                "SELECT enabled FROM connection_users WHERE user_id = ?1",
                [user_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        if !enabled {
            return Ok(BTreeSet::new());
        }
        let mut statement = connection.prepare(
            "SELECT DISTINCT rp.permission
             FROM connection_user_roles ur
             JOIN connection_role_permissions rp ON rp.role_id = ur.role_id
             WHERE ur.user_id = ?1",
        )?;
        statement
            .query_map([user_id.as_str()], |row| row.get::<_, String>(0))?
            .map(|value| decode(&value?))
            .collect()
    }

    fn insert_connection(&self, connection: &BrokerConnection) -> Result<(), RepositoryError> {
        write_connection(&self.connection()?, connection, false)
    }

    fn update_connection(&self, connection: &BrokerConnection) -> Result<(), RepositoryError> {
        write_connection(&self.connection()?, connection, true)
    }

    fn connection(&self, id: &ConnectionId) -> Result<Option<BrokerConnection>, RepositoryError> {
        self.connection()?
            .query_row(
                &(CONNECTION_SELECT.to_owned() + " WHERE connection_id = ?1"),
                [id.as_str()],
                read_connection,
            )
            .optional()
            .map_err(Into::into)
    }

    fn list_connections(&self) -> Result<Vec<BrokerConnection>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare(&(CONNECTION_SELECT.to_owned() + " ORDER BY created_at_unix_ms"))?;
        statement
            .query_map([], read_connection)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn replace_accounts(
        &self,
        connection_id: &ConnectionId,
        accounts: &[BrokerAccount],
    ) -> Result<(), RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE broker_accounts SET accessible = 0 WHERE connection_id = ?1",
            [connection_id.as_str()],
        )?;
        for account in accounts {
            if &account.connection_id != connection_id {
                return Err(RepositoryError::IdentityMismatch);
            }
            transaction.execute(
                "INSERT INTO broker_accounts (
                    connection_id, provider, environment, provider_account_id, display_name,
                    account_type, account_status, access_level, opened_at_unix_ms,
                    closed_at_unix_ms, accessible, capabilities_json, discovered_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(connection_id, provider_account_id) DO UPDATE SET
                    provider = excluded.provider,
                    environment = excluded.environment,
                    display_name = excluded.display_name,
                    account_type = excluded.account_type,
                    account_status = excluded.account_status,
                    access_level = excluded.access_level,
                    opened_at_unix_ms = excluded.opened_at_unix_ms,
                    closed_at_unix_ms = excluded.closed_at_unix_ms,
                    accessible = excluded.accessible,
                    capabilities_json = excluded.capabilities_json,
                    discovered_at_unix_ms = excluded.discovered_at_unix_ms",
                params![
                    connection_id.as_str(),
                    account.provider.as_str(),
                    encode(&account.environment)?,
                    account.provider_account_id,
                    account.display_name,
                    encode(&account.account_type)?,
                    encode(&account.status)?,
                    encode(&account.access_level)?,
                    account.opened_at_unix_ms,
                    account.closed_at_unix_ms,
                    account.accessible,
                    encode(&account.capabilities)?,
                    account.discovered_at_unix_ms,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn accounts(
        &self,
        connection_id: &ConnectionId,
    ) -> Result<Vec<BrokerAccount>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT connection_id, provider, environment, provider_account_id, display_name,
             account_type, account_status, access_level, opened_at_unix_ms, closed_at_unix_ms,
             accessible, capabilities_json, discovered_at_unix_ms FROM broker_accounts
             WHERE connection_id = ?1 ORDER BY provider_account_id",
        )?;
        statement
            .query_map([connection_id.as_str()], |row| {
                Ok(BrokerAccount {
                    connection_id: ConnectionId::parse(row.get::<_, String>(0)?)
                        .map_err(conversion_error)?,
                    provider: ProviderId::new(row.get::<_, String>(1)?)
                        .map_err(conversion_error)?,
                    environment: decode_row(row, 2)?,
                    provider_account_id: row.get(3)?,
                    display_name: row.get(4)?,
                    account_type: decode_row(row, 5)?,
                    status: decode_row(row, 6)?,
                    access_level: decode_row(row, 7)?,
                    opened_at_unix_ms: row.get(8)?,
                    closed_at_unix_ms: row.get(9)?,
                    accessible: row.get(10)?,
                    capabilities: decode_row(row, 11)?,
                    discovered_at_unix_ms: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn put_binding(&self, binding: &BrokerAccountBinding) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "INSERT INTO broker_account_bindings (
                binding_id, connection_id, provider, environment, provider_account_id,
                vox_account_id, enabled, created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                binding.id.as_str(),
                binding.connection_id.as_str(),
                binding.provider.as_str(),
                encode(&binding.environment)?,
                binding.provider_account_id,
                binding.vox_account_id.as_str(),
                binding.enabled,
                binding.created_at_unix_ms,
                binding.updated_at_unix_ms,
            ],
        )?;
        Ok(())
    }

    fn delete_binding(&self, binding_id: &BindingId) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "DELETE FROM broker_account_bindings WHERE binding_id = ?1",
            [binding_id.as_str()],
        )?;
        Ok(())
    }

    fn set_binding_enabled(
        &self,
        binding_id: &BindingId,
        enabled: bool,
        now_unix_ms: i64,
    ) -> Result<(), RepositoryError> {
        let changed = self.connection()?.execute(
            "UPDATE broker_account_bindings SET enabled = ?2, updated_at_unix_ms = ?3
             WHERE binding_id = ?1",
            params![binding_id.as_str(), enabled, now_unix_ms],
        )?;
        if changed != 1 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }

    fn bindings(
        &self,
        connection_id: &ConnectionId,
    ) -> Result<Vec<BrokerAccountBinding>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT binding_id, connection_id, provider, environment, provider_account_id,
             vox_account_id, enabled, created_at_unix_ms, updated_at_unix_ms
             FROM broker_account_bindings WHERE connection_id = ?1
             ORDER BY created_at_unix_ms",
        )?;
        statement
            .query_map([connection_id.as_str()], |row| {
                Ok(BrokerAccountBinding {
                    id: BindingId::parse(row.get::<_, String>(0)?).map_err(conversion_error)?,
                    connection_id: ConnectionId::parse(row.get::<_, String>(1)?)
                        .map_err(conversion_error)?,
                    provider: ProviderId::new(row.get::<_, String>(2)?)
                        .map_err(conversion_error)?,
                    environment: decode_row(row, 3)?,
                    provider_account_id: row.get(4)?,
                    vox_account_id: crate::model::VoxAccountId::parse(row.get::<_, String>(5)?)
                        .map_err(conversion_error)?,
                    enabled: row.get(6)?,
                    created_at_unix_ms: row.get(7)?,
                    updated_at_unix_ms: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn authorization(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<Option<ExecutionAuthorization>, RepositoryError> {
        self.connection()?
            .query_row(
                "SELECT connection_id, provider_account_id, authorization_mode,
                 changed_by, changed_at_unix_ms FROM execution_authorizations
                 WHERE connection_id = ?1 AND provider_account_id = ?2",
                params![connection_id.as_str(), provider_account_id],
                |row| {
                    Ok(ExecutionAuthorization {
                        connection_id: ConnectionId::parse(row.get::<_, String>(0)?)
                            .map_err(conversion_error)?,
                        provider_account_id: row.get(1)?,
                        mode: decode_row(row, 2)?,
                        changed_by: UserId::parse(row.get::<_, String>(3)?)
                            .map_err(conversion_error)?,
                        changed_at_unix_ms: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn put_authorization(
        &self,
        authorization: &ExecutionAuthorization,
    ) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "INSERT INTO execution_authorizations (
                connection_id, provider_account_id, authorization_mode,
                changed_by, changed_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(connection_id, provider_account_id) DO UPDATE SET
                authorization_mode = excluded.authorization_mode,
                changed_by = excluded.changed_by,
                changed_at_unix_ms = excluded.changed_at_unix_ms",
            params![
                authorization.connection_id.as_str(),
                authorization.provider_account_id,
                encode(&authorization.mode)?,
                authorization.changed_by.as_str(),
                authorization.changed_at_unix_ms,
            ],
        )?;
        Ok(())
    }

    fn append_audit(&self, record: &AuditRecord) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "INSERT INTO connection_audit (
                actor, action, target_ref, previous_state, new_state, correlation_id,
                occurred_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.actor.as_str(),
                record.action,
                record.target_ref,
                record.previous_state,
                record.new_state,
                record.correlation_id,
                record.occurred_at_unix_ms,
            ],
        )?;
        Ok(())
    }

    fn audit_records(&self) -> Result<Vec<AuditRecord>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT actor, action, target_ref, previous_state, new_state, correlation_id,
             occurred_at_unix_ms FROM connection_audit ORDER BY audit_id",
        )?;
        statement
            .query_map([], |row| {
                Ok(AuditRecord {
                    actor: UserId::parse(row.get::<_, String>(0)?).map_err(conversion_error)?,
                    action: row.get(1)?,
                    target_ref: row.get(2)?,
                    previous_state: row.get(3)?,
                    new_state: row.get(4)?,
                    correlation_id: row.get(5)?,
                    occurred_at_unix_ms: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn delete_connection(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
        self.connection()?.execute(
            "DELETE FROM broker_connections WHERE connection_id = ?1",
            [id.as_str()],
        )?;
        Ok(())
    }
}

fn write_connection(
    connection: &Connection,
    value: &BrokerConnection,
    update: bool,
) -> Result<(), RepositoryError> {
    let changed = if update {
        connection.execute(
            "UPDATE broker_connections SET label = ?2, credential_fingerprint = ?3, enabled = ?4,
             credential_status = ?5, credential_class = ?6, credential_scope = ?7,
             health_json = ?8, capabilities_json = ?9, updated_at_unix_ms = ?10
             WHERE connection_id = ?1",
            params![
                value.id.as_str(),
                value.label,
                value.credential_fingerprint,
                value.enabled,
                encode(&value.credential_status)?,
                encode(&value.credential_class)?,
                encode(&value.credential_scope)?,
                encode(&value.health)?,
                encode(&value.capabilities)?,
                value.updated_at_unix_ms,
            ],
        )?
    } else {
        connection.execute(
            "INSERT INTO broker_connections (
                connection_id, provider, environment, credential_ref, label,
                credential_fingerprint, credential_status, credential_class, credential_scope,
                enabled, health_json, capabilities_json,
                created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                value.id.as_str(),
                value.provider.as_str(),
                encode(&value.environment)?,
                value.credential_ref.as_str(),
                value.label,
                value.credential_fingerprint,
                encode(&value.credential_status)?,
                encode(&value.credential_class)?,
                encode(&value.credential_scope)?,
                value.enabled,
                encode(&value.health)?,
                encode(&value.capabilities)?,
                value.created_at_unix_ms,
                value.updated_at_unix_ms,
            ],
        )?
    };
    if changed != 1 {
        return Err(RepositoryError::NotFound);
    }
    Ok(())
}

const CONNECTION_SELECT: &str = "SELECT connection_id, provider, environment, credential_ref,
    label, credential_fingerprint, credential_status, credential_class, credential_scope,
    enabled, health_json, capabilities_json,
    created_at_unix_ms, updated_at_unix_ms FROM broker_connections";

fn read_connection(row: &rusqlite::Row<'_>) -> rusqlite::Result<BrokerConnection> {
    Ok(BrokerConnection {
        id: ConnectionId::parse(row.get::<_, String>(0)?).map_err(conversion_error)?,
        provider: crate::model::ProviderId::new(row.get::<_, String>(1)?)
            .map_err(conversion_error)?,
        environment: decode_row(row, 2)?,
        credential_ref: crate::model::CredentialRef::parse(row.get::<_, String>(3)?)
            .map_err(conversion_error)?,
        label: row.get(4)?,
        credential_fingerprint: row.get(5)?,
        credential_status: decode_row(row, 6)?,
        credential_class: decode_row(row, 7)?,
        credential_scope: decode_row(row, 8)?,
        enabled: row.get(9)?,
        health: decode_row(row, 10)?,
        capabilities: decode_row(row, 11)?,
        created_at_unix_ms: row.get(12)?,
        updated_at_unix_ms: row.get(13)?,
    })
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String, RepositoryError> {
    serde_json::to_string(value).map_err(|error| RepositoryError::Corrupt(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, RepositoryError> {
    serde_json::from_str(value).map_err(|error| RepositoryError::Corrupt(error.to_string()))
}

fn decode_row<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let value = row.get::<_, String>(index)?;
    decode(&value).map_err(conversion_error)
}

fn conversion_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RepositoryError {
    #[error("connection persistence failed: {0}")]
    Persistence(String),
    #[error("connection record not found")]
    NotFound,
    #[error("connection/account identity mismatch")]
    IdentityMismatch,
    #[error("connection persistence is corrupt: {0}")]
    Corrupt(String),
}

impl From<rusqlite::Error> for RepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Persistence(error.to_string())
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS connection_users (
    user_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    enabled INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS connection_roles (
    role_id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;
CREATE TABLE IF NOT EXISTS connection_role_permissions (
    role_id TEXT NOT NULL REFERENCES connection_roles(role_id) ON DELETE CASCADE,
    permission TEXT NOT NULL,
    PRIMARY KEY (role_id, permission)
) STRICT;
CREATE TABLE IF NOT EXISTS connection_user_roles (
    user_id TEXT NOT NULL REFERENCES connection_users(user_id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES connection_roles(role_id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, role_id)
) STRICT;
CREATE TABLE IF NOT EXISTS broker_connections (
    connection_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    environment TEXT NOT NULL,
    credential_ref TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    credential_fingerprint TEXT NOT NULL,
    credential_status TEXT NOT NULL,
    credential_class TEXT NOT NULL,
    credential_scope TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    health_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS broker_accounts (
    connection_id TEXT NOT NULL REFERENCES broker_connections(connection_id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    environment TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    display_name TEXT,
    account_type TEXT NOT NULL,
    account_status TEXT NOT NULL,
    access_level TEXT NOT NULL,
    opened_at_unix_ms INTEGER,
    closed_at_unix_ms INTEGER,
    accessible INTEGER NOT NULL,
    capabilities_json TEXT NOT NULL,
    discovered_at_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (connection_id, provider_account_id)
) STRICT;
CREATE TABLE IF NOT EXISTS broker_account_bindings (
    binding_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    environment TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    vox_account_id TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL,
    FOREIGN KEY (connection_id, provider_account_id)
        REFERENCES broker_accounts(connection_id, provider_account_id) ON DELETE CASCADE,
    UNIQUE (connection_id, provider_account_id)
) STRICT;
CREATE TABLE IF NOT EXISTS execution_authorizations (
    connection_id TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    authorization_mode TEXT NOT NULL,
    changed_by TEXT NOT NULL REFERENCES connection_users(user_id),
    changed_at_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (connection_id, provider_account_id),
    FOREIGN KEY (connection_id, provider_account_id)
        REFERENCES broker_accounts(connection_id, provider_account_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE IF NOT EXISTS connection_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor TEXT NOT NULL REFERENCES connection_users(user_id),
    action TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    previous_state TEXT,
    new_state TEXT,
    correlation_id TEXT NOT NULL,
    occurred_at_unix_ms INTEGER NOT NULL
) STRICT;
";
