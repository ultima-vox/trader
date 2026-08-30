use std::collections::BTreeSet;

use async_trait::async_trait;
use thiserror::Error;

use crate::model::{
    AccountTarget, AuditRecord, BindingId, BrokerAccessLevel, BrokerAccount, BrokerAccountBinding,
    BrokerAccountStatus, BrokerAccountType, BrokerConnection, BrokerEnvironment,
    ConnectionCapability, ConnectionHealth, ConnectionHealthReason, ConnectionHealthState,
    ConnectionId, CredentialClass, CredentialRef, CredentialScope, CredentialStatus,
    ExecutionAuthorization, ExecutionAuthorizationMode, ModelError, Permission, ProviderId, Role,
    RoleId, UserId, VoxAccountId, safe_text,
};
use crate::repository::{ConnectionRepository, RepositoryError};
use crate::secret::{CredentialContext, SecretBytes, SecretStore, SecretStoreError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityContext {
    pub actor: UserId,
    pub correlation_id: String,
    pub now_unix_ms: i64,
}

impl SecurityContext {
    pub fn new(
        actor: UserId,
        correlation_id: impl Into<String>,
        now_unix_ms: i64,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            actor,
            correlation_id: safe_text(correlation_id, "correlation_id", 256)?,
            now_unix_ms,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateConnectionRequest {
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderAccountFact {
    pub provider_account_id: String,
    pub display_name: Option<String>,
    pub account_type: BrokerAccountType,
    pub status: BrokerAccountStatus,
    pub access_level: BrokerAccessLevel,
    pub opened_at_unix_ms: Option<i64>,
    pub closed_at_unix_ms: Option<i64>,
    pub accessible: bool,
    pub capabilities: BTreeSet<ConnectionCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDiscovery {
    pub credential_class: CredentialClass,
    pub credential_scope: CredentialScope,
    pub connection_capabilities: BTreeSet<ConnectionCapability>,
    pub accounts: Vec<ProviderAccountFact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderErrorKind {
    InvalidCredential,
    InsufficientPermission,
    WrongEnvironment,
    ProviderUnavailable,
    ExpiredOrInactive,
    AccountAccessChanged,
    UnsupportedProvider,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("provider validation failed: {kind:?}: {safe_message}")]
pub struct ProviderError {
    kind: ProviderErrorKind,
    safe_message: String,
}

impl ProviderError {
    #[must_use]
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        let safe_message = safe_text(message, "provider_error", 256)
            .unwrap_or_else(|_| "provider validation failed".to_owned());
        Self { kind, safe_message }
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderErrorKind {
        self.kind
    }
}

#[async_trait]
pub trait BrokerProviderPort: Send + Sync {
    async fn validate_and_discover(
        &self,
        provider: &ProviderId,
        environment: BrokerEnvironment,
        credential: &SecretBytes,
    ) -> Result<ProviderDiscovery, ProviderError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionPurpose {
    SandboxMutation,
    ProductionManual,
    ProductionAutomated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadAccessGrant {
    pub target: AccountTarget,
    pub credential_ref: CredentialRef,
    pub capabilities: BTreeSet<ConnectionCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAccessGrant {
    pub target: AccountTarget,
    pub credential_ref: CredentialRef,
    pub capabilities: BTreeSet<ConnectionCapability>,
    pub mode: ExecutionAuthorizationMode,
    pub authorization_revision: u64,
}

pub trait BrokerCredentialClientFactory {
    type ReadClient;
    type ExecutionClient;

    fn build_read_client(
        &self,
        target: AccountTarget,
        credential: SecretBytes,
    ) -> Result<Self::ReadClient, ProviderError>;

    fn build_execution_client(
        &self,
        target: AccountTarget,
        credential: SecretBytes,
        authorization_revision: u64,
    ) -> Result<Self::ExecutionClient, ProviderError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialRotationOutcome {
    pub connection: BrokerConnection,
    pub reconnect_required: bool,
}

pub struct ConnectionService<R, S, P> {
    repository: R,
    secret_store: S,
    provider: P,
}

impl<R, S, P> ConnectionService<R, S, P>
where
    R: ConnectionRepository,
    S: SecretStore,
    P: BrokerProviderPort,
{
    #[must_use]
    pub const fn new(repository: R, secret_store: S, provider: P) -> Self {
        Self {
            repository,
            secret_store,
            provider,
        }
    }

    pub async fn create_connection(
        &self,
        security: &SecurityContext,
        request: CreateConnectionRequest,
        credential: SecretBytes,
    ) -> Result<BrokerConnection, ServiceError> {
        self.require(security, Permission::ManageCredentials)?;
        let label = safe_text(request.label, "connection_label", 128)?;
        let context = CredentialContext {
            provider: request.provider.clone(),
            environment: request.environment,
        };
        let discovery = self
            .provider
            .validate_and_discover(&request.provider, request.environment, &credential)
            .await?;
        validate_discovery(&discovery)?;

        let connection_id = ConnectionId::new();
        let credential_ref = CredentialRef::new();
        let pending = BrokerConnection {
            id: connection_id.clone(),
            provider: request.provider.clone(),
            environment: request.environment,
            credential_ref: credential_ref.clone(),
            label: label.clone(),
            credential_fingerprint: "PENDING".to_owned(),
            credential_status: CredentialStatus::PendingValidation,
            credential_class: CredentialClass::Unknown,
            credential_scope: CredentialScope::NotConfirmed,
            enabled: false,
            health: ConnectionHealth {
                state: ConnectionHealthState::Validating,
                checked_at_unix_ms: Some(security.now_unix_ms),
                provider: request.provider.clone(),
                environment: request.environment,
                reason_code: ConnectionHealthReason::None,
                safe_detail: None,
                retryable: false,
            },
            capabilities: BTreeSet::new(),
            created_at_unix_ms: security.now_unix_ms,
            updated_at_unix_ms: security.now_unix_ms,
        };
        let pending_audit = self.audit_record(
            security,
            "CONNECTION_ONBOARDING_STARTED",
            connection_id.as_str(),
            None,
            Some("PENDING_VALIDATION"),
        )?;
        self.repository
            .insert_connection_with_audit(&pending, &pending_audit)?;
        let fingerprint =
            self.secret_store
                .put(&credential_ref, &context, credential, security.now_unix_ms)?;
        let connection = BrokerConnection {
            id: connection_id.clone(),
            provider: request.provider,
            environment: request.environment,
            credential_ref: credential_ref.clone(),
            label,
            credential_fingerprint: fingerprint,
            credential_status: CredentialStatus::Valid,
            credential_class: discovery.credential_class,
            credential_scope: discovery.credential_scope,
            enabled: true,
            health: ConnectionHealth {
                state: ConnectionHealthState::Healthy,
                checked_at_unix_ms: Some(security.now_unix_ms),
                provider: context.provider.clone(),
                environment: context.environment,
                reason_code: ConnectionHealthReason::None,
                safe_detail: None,
                retryable: false,
            },
            capabilities: discovery.connection_capabilities,
            created_at_unix_ms: security.now_unix_ms,
            updated_at_unix_ms: security.now_unix_ms,
        };
        let accounts = materialize_accounts(&connection, discovery.accounts, security.now_unix_ms);
        let authorizations = accounts
            .iter()
            .map(|account| ExecutionAuthorization {
                connection_id: connection_id.clone(),
                provider_account_id: account.provider_account_id.clone(),
                mode: ExecutionAuthorizationMode::Disabled,
                authorization_revision: 1,
                changed_by: security.actor.clone(),
                changed_at_unix_ms: security.now_unix_ms,
            })
            .collect::<Vec<_>>();
        let audits = vec![
            self.audit_record(
                security,
                "CONNECTION_CREATED",
                connection_id.as_str(),
                None,
                Some("ENABLED"),
            )?,
            self.audit_record(
                security,
                "CREDENTIAL_ADDED",
                credential_ref.as_str(),
                None,
                Some("VALID"),
            )?,
        ];
        if let Err(error) =
            self.repository
                .insert_onboarding(&connection, &accounts, &authorizations, &audits)
        {
            let _ignored = self.secret_store.delete(&credential_ref);
            return Err(error.into());
        }
        Ok(connection)
    }

    pub fn list_connections(
        &self,
        security: &SecurityContext,
    ) -> Result<Vec<BrokerConnection>, ServiceError> {
        self.require(security, Permission::ViewConnectionMetadata)?;
        self.repository.list_connections().map_err(Into::into)
    }

    pub fn discovered_accounts(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
    ) -> Result<Vec<BrokerAccount>, ServiceError> {
        self.require(security, Permission::ViewConnectionMetadata)?;
        self.connection(connection_id)?;
        self.repository.accounts(connection_id).map_err(Into::into)
    }

    pub fn account_bindings(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
    ) -> Result<Vec<BrokerAccountBinding>, ServiceError> {
        self.require(security, Permission::ViewConnectionMetadata)?;
        self.connection(connection_id)?;
        self.repository.bindings(connection_id).map_err(Into::into)
    }

    pub fn execution_authorization(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<ExecutionAuthorization, ServiceError> {
        self.require(security, Permission::ViewConnectionMetadata)?;
        self.connection(connection_id)?;
        self.repository
            .authorization(connection_id, provider_account_id)?
            .ok_or(ServiceError::AccountNotFound)
    }

    pub async fn revalidate(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
    ) -> Result<BrokerConnection, ServiceError> {
        self.require(security, Permission::DiscoverAccounts)?;
        let mut connection = self.connection(connection_id)?;
        if !connection.enabled {
            return Err(ServiceError::ConnectionDisabled);
        }
        let context = credential_context(&connection);
        let credential = self
            .secret_store
            .get(&connection.credential_ref, &context)?;
        match self
            .provider
            .validate_and_discover(&connection.provider, connection.environment, &credential)
            .await
        {
            Ok(discovery) => {
                validate_discovery(&discovery)?;
                let accounts =
                    materialize_accounts(&connection, discovery.accounts, security.now_unix_ms);
                let old_accounts = self.repository.accounts(connection_id)?;
                let effective_accounts = with_disappeared_accounts(&old_accounts, &accounts);
                let access_changed = access_removed(&old_accounts, &accounts)
                    || self.authorization_conflict(connection_id, &effective_accounts)?;
                connection.capabilities = discovery.connection_capabilities;
                connection.credential_status = CredentialStatus::Valid;
                connection.credential_class = discovery.credential_class;
                connection.credential_scope = discovery.credential_scope;
                connection.health = ConnectionHealth {
                    state: if access_changed {
                        ConnectionHealthState::AccountAccessChanged
                    } else {
                        ConnectionHealthState::Healthy
                    },
                    checked_at_unix_ms: Some(security.now_unix_ms),
                    provider: connection.provider.clone(),
                    environment: connection.environment,
                    reason_code: if access_changed {
                        ConnectionHealthReason::AccountAccessChanged
                    } else {
                        ConnectionHealthReason::None
                    },
                    safe_detail: None,
                    retryable: false,
                };
                connection.updated_at_unix_ms = security.now_unix_ms;
                // Close admission before publishing reduced broker facts.
                self.repository.update_connection(&connection)?;
                self.repository.replace_accounts(connection_id, &accounts)?;
                let persisted_accounts = self.repository.accounts(connection_id)?;
                self.revoke_invalid_authorizations(security, &connection, &persisted_accounts)?;
            }
            Err(error) => {
                connection.credential_status = match error.kind() {
                    ProviderErrorKind::ExpiredOrInactive => CredentialStatus::ExpiredOrInactive,
                    ProviderErrorKind::InvalidCredential | ProviderErrorKind::WrongEnvironment => {
                        CredentialStatus::Invalid
                    }
                    _ => connection.credential_status,
                };
                connection.health = health_from_provider_error(
                    &error,
                    &connection.provider,
                    connection.environment,
                    security.now_unix_ms,
                );
                connection.updated_at_unix_ms = security.now_unix_ms;
                self.repository.update_connection(&connection)?;
                self.audit(
                    security,
                    "CONNECTION_REVALIDATION_FAILED",
                    connection_id.as_str(),
                    None,
                    Some(health_name(connection.health.state)),
                )?;
                return Err(error.into());
            }
        }
        self.repository.update_connection(&connection)?;
        self.audit(
            security,
            "CONNECTION_REVALIDATED",
            connection_id.as_str(),
            None,
            Some(health_name(connection.health.state)),
        )?;
        Ok(connection)
    }

    pub async fn rotate_credential(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
        new_credential: SecretBytes,
    ) -> Result<CredentialRotationOutcome, ServiceError> {
        self.require(security, Permission::ManageCredentials)?;
        let previous_connection = self.connection(connection_id)?;
        let discovery = self
            .provider
            .validate_and_discover(
                &previous_connection.provider,
                previous_connection.environment,
                &new_credential,
            )
            .await?;
        validate_discovery(&discovery)?;
        let context = credential_context(&previous_connection);
        let previous_credential = self
            .secret_store
            .get(&previous_connection.credential_ref, &context)?;
        let old_accounts = self.repository.accounts(connection_id)?;
        let accounts = materialize_accounts(
            &previous_connection,
            discovery.accounts.clone(),
            security.now_unix_ms,
        );
        let effective_accounts = with_disappeared_accounts(&old_accounts, &accounts);
        let access_changed = access_removed(&old_accounts, &accounts)
            || self.authorization_conflict(connection_id, &effective_accounts)?;

        let mut gate = previous_connection.clone();
        gate.health = ConnectionHealth {
            state: ConnectionHealthState::Validating,
            checked_at_unix_ms: Some(security.now_unix_ms),
            provider: gate.provider.clone(),
            environment: gate.environment,
            reason_code: ConnectionHealthReason::None,
            safe_detail: Some("credential rotation in progress".to_owned()),
            retryable: false,
        };
        gate.updated_at_unix_ms = security.now_unix_ms;
        self.repository.update_connection(&gate)?;

        let new_fingerprint = match self.secret_store.rotate(
            &previous_connection.credential_ref,
            &context,
            new_credential,
            security.now_unix_ms,
        ) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                let _ignored = self.repository.update_connection(&previous_connection);
                return Err(error.into());
            }
        };

        let mut connection = previous_connection.clone();
        connection.credential_fingerprint = new_fingerprint;
        connection.capabilities = discovery.connection_capabilities;
        connection.credential_status = CredentialStatus::Valid;
        connection.credential_class = discovery.credential_class;
        connection.credential_scope = discovery.credential_scope;
        connection.updated_at_unix_ms = security.now_unix_ms;
        connection.health = ConnectionHealth {
            state: if access_changed {
                ConnectionHealthState::AccountAccessChanged
            } else {
                ConnectionHealthState::Healthy
            },
            checked_at_unix_ms: Some(security.now_unix_ms),
            provider: connection.provider.clone(),
            environment: connection.environment,
            reason_code: if access_changed {
                ConnectionHealthReason::AccountAccessChanged
            } else {
                ConnectionHealthReason::None
            },
            safe_detail: None,
            retryable: false,
        };
        let persistence = (|| -> Result<(), ServiceError> {
            self.repository.replace_accounts(connection_id, &accounts)?;
            self.repository.update_connection(&connection)?;
            let persisted_accounts = self.repository.accounts(connection_id)?;
            self.revoke_invalid_authorizations(security, &connection, &persisted_accounts)?;
            self.audit(
                security,
                "CREDENTIAL_ROTATED_RECONNECT_REQUIRED",
                connection_id.as_str(),
                Some(&previous_connection.credential_fingerprint),
                Some(&connection.credential_fingerprint),
            )
        })();
        if let Err(error) = persistence {
            let secret_restored = self
                .secret_store
                .rotate(
                    &previous_connection.credential_ref,
                    &context,
                    previous_credential,
                    security.now_unix_ms,
                )
                .is_ok();
            let accounts_restored = self
                .repository
                .replace_accounts(connection_id, &old_accounts)
                .is_ok();
            let connection_restored = self
                .repository
                .update_connection(&previous_connection)
                .is_ok();
            if !secret_restored || !accounts_restored || !connection_restored {
                let _ignored = self
                    .secret_store
                    .disable(&previous_connection.credential_ref);
                return Err(ServiceError::CredentialRotationCompensationFailed);
            }
            return Err(error);
        }
        Ok(CredentialRotationOutcome {
            connection,
            reconnect_required: true,
        })
    }

    pub fn disable_connection(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
    ) -> Result<BrokerConnection, ServiceError> {
        self.require(security, Permission::DisableDeleteConnection)?;
        let mut connection = self.connection(connection_id)?;
        if connection.credential_status != CredentialStatus::PendingDisable {
            connection.enabled = false;
            connection.credential_status = CredentialStatus::PendingDisable;
            connection.health = ConnectionHealth {
                state: ConnectionHealthState::Disabled,
                checked_at_unix_ms: Some(security.now_unix_ms),
                provider: connection.provider.clone(),
                environment: connection.environment,
                reason_code: ConnectionHealthReason::DisabledByOperator,
                safe_detail: Some("credential disable in progress".to_owned()),
                retryable: false,
            };
            connection.updated_at_unix_ms = security.now_unix_ms;
            let started = self.audit_record(
                security,
                "CONNECTION_DISABLE_STARTED",
                connection_id.as_str(),
                Some("ENABLED"),
                Some("PENDING_DISABLE"),
            )?;
            self.repository
                .update_connection_with_audit(&connection, &started)?;
        }
        self.secret_store.disable(&connection.credential_ref)?;
        connection.credential_status = CredentialStatus::Disabled;
        connection.health = ConnectionHealth {
            state: ConnectionHealthState::Disabled,
            checked_at_unix_ms: Some(security.now_unix_ms),
            provider: connection.provider.clone(),
            environment: connection.environment,
            reason_code: ConnectionHealthReason::DisabledByOperator,
            safe_detail: Some("disabled by authorized operator".to_owned()),
            retryable: false,
        };
        connection.updated_at_unix_ms = security.now_unix_ms;
        let audits = [
            self.audit_record(
                security,
                "CREDENTIAL_DISABLED",
                connection.credential_ref.as_str(),
                Some("VALID"),
                Some("DISABLED"),
            )?,
            self.audit_record(
                security,
                "CONNECTION_DISABLED",
                connection_id.as_str(),
                Some("ENABLED"),
                Some("DISABLED"),
            )?,
        ];
        self.repository
            .update_connection_with_audits(&connection, &audits)?;
        Ok(connection)
    }

    pub fn delete_connection(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
    ) -> Result<(), ServiceError> {
        self.require(security, Permission::DisableDeleteConnection)?;
        let connection = self.connection(connection_id)?;
        if connection.enabled && connection.credential_status != CredentialStatus::PendingDelete {
            return Err(ServiceError::DisableBeforeDelete);
        }
        let mut tombstone = connection.clone();
        tombstone.enabled = false;
        tombstone.credential_status = CredentialStatus::PendingDelete;
        tombstone.health = ConnectionHealth {
            state: ConnectionHealthState::Disabled,
            checked_at_unix_ms: Some(security.now_unix_ms),
            provider: tombstone.provider.clone(),
            environment: tombstone.environment,
            reason_code: ConnectionHealthReason::DisabledByOperator,
            safe_detail: Some("delete in progress".to_owned()),
            retryable: false,
        };
        tombstone.updated_at_unix_ms = security.now_unix_ms;
        let started = self.audit_record(
            security,
            "CONNECTION_DELETE_STARTED",
            connection_id.as_str(),
            Some("DISABLED"),
            Some("PENDING_DELETE"),
        )?;
        self.repository
            .begin_connection_delete_with_audit(&tombstone, &started)?;
        match self.secret_store.delete(&connection.credential_ref) {
            Ok(()) | Err(SecretStoreError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        let deleted = self.audit_record(
            security,
            "CONNECTION_DELETED",
            connection_id.as_str(),
            Some("PENDING_DELETE"),
            Some("DELETED"),
        )?;
        self.repository
            .finalize_connection_delete(connection_id, &deleted)?;
        Ok(())
    }

    pub fn bind_account(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
        provider_account_id: impl Into<String>,
        vox_account_id: VoxAccountId,
    ) -> Result<BrokerAccountBinding, ServiceError> {
        self.require(security, Permission::BindAccounts)?;
        let connection = self.connection(connection_id)?;
        let provider_account_id = safe_text(provider_account_id, "provider_account_id", 256)?;
        let account = self
            .repository
            .accounts(connection_id)?
            .into_iter()
            .find(|account| account.provider_account_id == provider_account_id)
            .ok_or(ServiceError::AccountNotFound)?;
        if !account.accessible {
            return Err(ServiceError::AccountUnavailable);
        }
        let binding = BrokerAccountBinding {
            id: BindingId::new(),
            connection_id: connection_id.clone(),
            provider: connection.provider,
            environment: connection.environment,
            provider_account_id,
            vox_account_id,
            enabled: true,
            created_at_unix_ms: security.now_unix_ms,
            updated_at_unix_ms: security.now_unix_ms,
        };
        let audit = self.audit_record(
            security,
            "ACCOUNT_BOUND",
            binding.id.as_str(),
            None,
            Some(connection_id.as_str()),
        )?;
        self.repository.put_binding_with_audit(&binding, &audit)?;
        Ok(binding)
    }

    pub fn unbind_account(
        &self,
        security: &SecurityContext,
        binding_id: &BindingId,
    ) -> Result<(), ServiceError> {
        self.require(security, Permission::BindAccounts)?;
        let audit = self.audit_record(
            security,
            "ACCOUNT_UNBOUND",
            binding_id.as_str(),
            Some("BOUND"),
            Some("UNBOUND"),
        )?;
        self.repository
            .delete_binding_with_audit(binding_id, &audit)?;
        Ok(())
    }

    pub fn replace_role(
        &self,
        security: &SecurityContext,
        mut role: Role,
    ) -> Result<(), ServiceError> {
        self.require(security, Permission::SecurityAdmin)?;
        role.name = safe_text(role.name, "role_name", 128)?;
        let audit = self.audit_record(
            security,
            "ROLE_PERMISSIONS_CHANGED",
            role.id.as_str(),
            None,
            Some("REPLACED"),
        )?;
        self.repository.put_role_with_audit(&role, &audit)?;
        Ok(())
    }

    pub fn grant_role(
        &self,
        security: &SecurityContext,
        user_id: &UserId,
        role_id: &RoleId,
    ) -> Result<(), ServiceError> {
        self.require(security, Permission::SecurityAdmin)?;
        let audit = self.audit_record(
            security,
            "ROLE_ASSIGNED",
            user_id.as_str(),
            None,
            Some(role_id.as_str()),
        )?;
        self.repository
            .grant_role_with_audit(user_id, role_id, &audit)?;
        Ok(())
    }

    pub fn disable_binding(
        &self,
        security: &SecurityContext,
        binding_id: &BindingId,
    ) -> Result<(), ServiceError> {
        self.require(security, Permission::BindAccounts)?;
        let audit = self.audit_record(
            security,
            "ACCOUNT_BINDING_DISABLED",
            binding_id.as_str(),
            Some("ENABLED"),
            Some("DISABLED"),
        )?;
        self.repository.set_binding_enabled_with_audit(
            binding_id,
            false,
            security.now_unix_ms,
            &audit,
        )?;
        Ok(())
    }

    pub fn set_execution_authorization(
        &self,
        security: &SecurityContext,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        mode: ExecutionAuthorizationMode,
    ) -> Result<ExecutionAuthorization, ServiceError> {
        let connection = self.connection(connection_id)?;
        let previous = self
            .repository
            .authorization(connection_id, provider_account_id)?
            .ok_or(ServiceError::AccountNotFound)?;
        self.require_authorization_transition(
            security,
            connection.environment,
            previous.mode,
            mode,
        )?;
        let account = self
            .repository
            .accounts(connection_id)?
            .into_iter()
            .find(|account| account.provider_account_id == provider_account_id)
            .ok_or(ServiceError::AccountNotFound)?;
        let binding = self
            .repository
            .bindings(connection_id)?
            .into_iter()
            .find(|binding| binding.provider_account_id == provider_account_id && binding.enabled)
            .ok_or(ServiceError::AccountBindingMismatch)?;
        if mode != ExecutionAuthorizationMode::Disabled {
            if !connection.enabled || connection.health.state != ConnectionHealthState::Healthy {
                return Err(ServiceError::ConnectionUnavailable);
            }
            if !account.accessible {
                return Err(ServiceError::AccountUnavailable);
            }
            if !account
                .capabilities
                .contains(&match connection.environment {
                    BrokerEnvironment::Production => {
                        ConnectionCapability::ProductionOrdersProviderAllowed
                    }
                    BrokerEnvironment::Sandbox => ConnectionCapability::SandboxOrders,
                })
            {
                return Err(ServiceError::ProviderDoesNotAllowProductionOrders);
            }
        }
        let authorization = ExecutionAuthorization {
            connection_id: connection_id.clone(),
            provider_account_id: provider_account_id.to_owned(),
            mode,
            authorization_revision: previous
                .authorization_revision
                .checked_add(1)
                .ok_or(ServiceError::AuthorizationRevisionOverflow)?,
            changed_by: security.actor.clone(),
            changed_at_unix_ms: security.now_unix_ms,
        };
        let audit = self.audit_record(
            security,
            "EXECUTION_AUTHORIZATION_CHANGED",
            binding.id.as_str(),
            Some(authorization_name(previous.mode)),
            Some(authorization_name(mode)),
        )?;
        self.repository.put_authorization_with_audit(
            &authorization,
            previous.authorization_revision,
            &audit,
        )?;
        Ok(authorization)
    }

    pub fn validate_read_access(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        vox_account_id: &VoxAccountId,
    ) -> Result<ReadAccessGrant, ServiceError> {
        let bound =
            self.bound_healthy_target(connection_id, provider_account_id, vox_account_id)?;
        let credential = self.secret_store.get(
            &bound.connection.credential_ref,
            &credential_context(&bound.connection),
        )?;
        drop(credential);
        Ok(ReadAccessGrant {
            target: bound.target,
            credential_ref: bound.connection.credential_ref,
            capabilities: bound.capabilities,
        })
    }

    pub fn validate_execution_access(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        vox_account_id: &VoxAccountId,
        purpose: ExecutionPurpose,
        expected_revision: Option<u64>,
    ) -> Result<ExecutionAccessGrant, ServiceError> {
        let bound =
            self.bound_healthy_target(connection_id, provider_account_id, vox_account_id)?;
        let authorization = self
            .repository
            .authorization(connection_id, provider_account_id)?
            .ok_or(ServiceError::ExecutionUnauthorized)?;
        if expected_revision
            .is_some_and(|revision| revision != authorization.authorization_revision)
        {
            return Err(ServiceError::StaleExecutionAuthorization);
        }
        if !execution_purpose_allowed(bound.connection.environment, authorization.mode, purpose) {
            return Err(ServiceError::ExecutionUnauthorized);
        }
        if !bound
            .capabilities
            .contains(&required_order_capability(bound.connection.environment))
        {
            return Err(ServiceError::ProviderDoesNotAllowProductionOrders);
        }
        let credential = self.secret_store.get(
            &bound.connection.credential_ref,
            &credential_context(&bound.connection),
        )?;
        drop(credential);
        Ok(ExecutionAccessGrant {
            target: bound.target,
            credential_ref: bound.connection.credential_ref,
            capabilities: bound.capabilities,
            mode: authorization.mode,
            authorization_revision: authorization.authorization_revision,
        })
    }

    pub fn validate_runtime_read_access(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<ReadAccessGrant, ServiceError> {
        let vox_account_id = self.bound_vox_account(connection_id, provider_account_id)?;
        self.validate_read_access(connection_id, provider_account_id, &vox_account_id)
    }

    pub fn validate_runtime_execution_access(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        purpose: ExecutionPurpose,
        expected_revision: Option<u64>,
    ) -> Result<ExecutionAccessGrant, ServiceError> {
        let vox_account_id = self.bound_vox_account(connection_id, provider_account_id)?;
        self.validate_execution_access(
            connection_id,
            provider_account_id,
            &vox_account_id,
            purpose,
            expected_revision,
        )
    }

    pub fn open_runtime_read_client<F: BrokerCredentialClientFactory>(
        &self,
        factory: &F,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<F::ReadClient, ServiceError> {
        let vox_account_id = self.bound_vox_account(connection_id, provider_account_id)?;
        let bound =
            self.bound_healthy_target(connection_id, provider_account_id, &vox_account_id)?;
        let credential = self.secret_store.get(
            &bound.connection.credential_ref,
            &credential_context(&bound.connection),
        )?;
        factory
            .build_read_client(bound.target, credential)
            .map_err(Into::into)
    }

    pub fn open_runtime_execution_client<F: BrokerCredentialClientFactory>(
        &self,
        factory: &F,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        purpose: ExecutionPurpose,
        expected_revision: u64,
    ) -> Result<F::ExecutionClient, ServiceError> {
        let grant = self.validate_runtime_execution_access(
            connection_id,
            provider_account_id,
            purpose,
            Some(expected_revision),
        )?;
        let connection = self.connection(connection_id)?;
        let credential = self
            .secret_store
            .get(&connection.credential_ref, &credential_context(&connection))?;
        factory
            .build_execution_client(grant.target, credential, grant.authorization_revision)
            .map_err(Into::into)
    }

    fn authorization_conflict(
        &self,
        connection_id: &ConnectionId,
        accounts: &[BrokerAccount],
    ) -> Result<bool, ServiceError> {
        for account in accounts {
            let mode = self
                .repository
                .authorization(connection_id, &account.provider_account_id)?
                .map(|authorization| authorization.mode)
                .unwrap_or(ExecutionAuthorizationMode::Disabled);
            if mode != ExecutionAuthorizationMode::Disabled
                && (!account.accessible
                    || !account
                        .capabilities
                        .contains(&required_order_capability(account.environment)))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn revoke_invalid_authorizations(
        &self,
        security: &SecurityContext,
        connection: &BrokerConnection,
        accounts: &[BrokerAccount],
    ) -> Result<(), ServiceError> {
        for account in accounts {
            let Some(previous) = self
                .repository
                .authorization(&connection.id, &account.provider_account_id)?
            else {
                continue;
            };
            if previous.mode == ExecutionAuthorizationMode::Disabled
                || (account.accessible
                    && account
                        .capabilities
                        .contains(&required_order_capability(account.environment)))
            {
                continue;
            }
            let target = self
                .repository
                .bindings(&connection.id)?
                .into_iter()
                .find(|binding| binding.provider_account_id == account.provider_account_id)
                .map_or_else(
                    || connection.id.as_str().to_owned(),
                    |binding| binding.id.as_str().to_owned(),
                );
            let audit = self.audit_record(
                security,
                "EXECUTION_AUTHORIZATION_REVOKED_PROVIDER_DOWNGRADE",
                &target,
                Some(authorization_name(previous.mode)),
                Some("DISABLED"),
            )?;
            self.repository.put_authorization_with_audit(
                &ExecutionAuthorization {
                    connection_id: connection.id.clone(),
                    provider_account_id: account.provider_account_id.clone(),
                    mode: ExecutionAuthorizationMode::Disabled,
                    authorization_revision: previous
                        .authorization_revision
                        .checked_add(1)
                        .ok_or(ServiceError::AuthorizationRevisionOverflow)?,
                    changed_by: security.actor.clone(),
                    changed_at_unix_ms: security.now_unix_ms,
                },
                previous.authorization_revision,
                &audit,
            )?;
        }
        Ok(())
    }

    fn connection(&self, id: &ConnectionId) -> Result<BrokerConnection, ServiceError> {
        self.repository
            .connection(id)?
            .ok_or(ServiceError::ConnectionNotFound)
    }

    fn bound_healthy_target(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        vox_account_id: &VoxAccountId,
    ) -> Result<BoundTarget, ServiceError> {
        let connection = self.connection(connection_id)?;
        if !connection.enabled
            || connection.health.state != ConnectionHealthState::Healthy
            || connection.credential_status == CredentialStatus::PendingDelete
        {
            return Err(ServiceError::ConnectionUnavailable);
        }
        let account = self
            .repository
            .accounts(connection_id)?
            .into_iter()
            .find(|account| account.provider_account_id == provider_account_id)
            .ok_or(ServiceError::AccountNotFound)?;
        if !account.accessible {
            return Err(ServiceError::AccountUnavailable);
        }
        let binding_exists = self
            .repository
            .bindings(connection_id)?
            .into_iter()
            .any(|binding| {
                binding.provider_account_id == provider_account_id
                    && &binding.vox_account_id == vox_account_id
                    && binding.enabled
            });
        if !binding_exists {
            return Err(ServiceError::AccountBindingMismatch);
        }
        Ok(BoundTarget {
            connection,
            target: AccountTarget {
                connection_id: connection_id.clone(),
                provider: account.provider.clone(),
                environment: account.environment,
                provider_account_id: provider_account_id.to_owned(),
                vox_account_id: vox_account_id.clone(),
            },
            capabilities: account.capabilities,
        })
    }

    fn bound_vox_account(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<VoxAccountId, ServiceError> {
        self.repository
            .bindings(connection_id)?
            .into_iter()
            .find(|binding| binding.provider_account_id == provider_account_id && binding.enabled)
            .map(|binding| binding.vox_account_id)
            .ok_or(ServiceError::AccountBindingMismatch)
    }

    fn require_authorization_transition(
        &self,
        security: &SecurityContext,
        environment: BrokerEnvironment,
        previous: ExecutionAuthorizationMode,
        mode: ExecutionAuthorizationMode,
    ) -> Result<(), ServiceError> {
        if authorization_rank(mode) < authorization_rank(previous)
            && (self.has(security, Permission::EmergencyHalt)?
                || self.has(security, Permission::DisableDeleteConnection)?
                || self.has(security, Permission::SecurityAdmin)?)
        {
            return Ok(());
        }
        match (environment, previous, mode) {
            (BrokerEnvironment::Production, _, ExecutionAuthorizationMode::AutomatedAllowed) => {
                self.require(security, Permission::EnableAutomatedProductionExecution)
            }
            (BrokerEnvironment::Production, _, ExecutionAuthorizationMode::ManualAllowed) => {
                self.require(security, Permission::SubmitProductionManualOrders)
            }
            (_, ExecutionAuthorizationMode::Disabled, ExecutionAuthorizationMode::Disabled) => {
                self.require(security, Permission::BindAccounts)
            }
            (BrokerEnvironment::Sandbox, _, _) => {
                self.require(security, Permission::SubmitSandboxOrders)
            }
            (BrokerEnvironment::Production, _, ExecutionAuthorizationMode::Disabled) => {
                self.require(security, Permission::EmergencyHalt)
            }
        }
    }

    fn has(
        &self,
        security: &SecurityContext,
        permission: Permission,
    ) -> Result<bool, ServiceError> {
        Ok(self
            .repository
            .permissions(&security.actor)?
            .contains(&permission))
    }

    fn require(
        &self,
        security: &SecurityContext,
        permission: Permission,
    ) -> Result<(), ServiceError> {
        if self
            .repository
            .permissions(&security.actor)?
            .contains(&permission)
        {
            Ok(())
        } else {
            Err(ServiceError::PermissionDenied(permission))
        }
    }

    fn audit(
        &self,
        security: &SecurityContext,
        action: &str,
        target_ref: &str,
        previous_state: Option<&str>,
        new_state: Option<&str>,
    ) -> Result<(), ServiceError> {
        let record = self.audit_record(security, action, target_ref, previous_state, new_state)?;
        self.repository.append_audit(&record)?;
        Ok(())
    }

    fn audit_record(
        &self,
        security: &SecurityContext,
        action: &str,
        target_ref: &str,
        previous_state: Option<&str>,
        new_state: Option<&str>,
    ) -> Result<AuditRecord, ServiceError> {
        let previous_state = previous_state
            .map(|value| safe_text(value, "audit_previous_state", 256))
            .transpose()?;
        let new_state = new_state
            .map(|value| safe_text(value, "audit_new_state", 256))
            .transpose()?;
        Ok(AuditRecord {
            actor: security.actor.clone(),
            action: safe_text(action, "audit_action", 128)?,
            target_ref: safe_text(target_ref, "audit_target", 256)?,
            previous_state,
            new_state,
            correlation_id: security.correlation_id.clone(),
            occurred_at_unix_ms: security.now_unix_ms,
        })
    }
}

fn validate_discovery(discovery: &ProviderDiscovery) -> Result<(), ServiceError> {
    let mut identities = BTreeSet::new();
    for account in &discovery.accounts {
        safe_text(
            account.provider_account_id.clone(),
            "provider_account_id",
            256,
        )?;
        if !identities.insert(account.provider_account_id.clone()) {
            return Err(ServiceError::DuplicateProviderAccount);
        }
        if let Some(name) = &account.display_name {
            safe_text(name.clone(), "account_display_name", 256)?;
        }
    }
    Ok(())
}

fn materialize_accounts(
    connection: &BrokerConnection,
    accounts: Vec<ProviderAccountFact>,
    now_unix_ms: i64,
) -> Vec<BrokerAccount> {
    accounts
        .into_iter()
        .map(|account| BrokerAccount {
            connection_id: connection.id.clone(),
            provider: connection.provider.clone(),
            environment: connection.environment,
            provider_account_id: account.provider_account_id,
            display_name: account.display_name,
            account_type: account.account_type,
            status: account.status,
            access_level: account.access_level,
            opened_at_unix_ms: account.opened_at_unix_ms,
            closed_at_unix_ms: account.closed_at_unix_ms,
            accessible: account.accessible,
            capabilities: account.capabilities,
            discovered_at_unix_ms: now_unix_ms,
        })
        .collect()
}

fn access_removed(previous: &[BrokerAccount], current: &[BrokerAccount]) -> bool {
    previous
        .iter()
        .filter(|account| account.accessible)
        .any(|old| {
            !current
                .iter()
                .any(|new| new.provider_account_id == old.provider_account_id && new.accessible)
        })
}

fn with_disappeared_accounts(
    previous: &[BrokerAccount],
    current: &[BrokerAccount],
) -> Vec<BrokerAccount> {
    let mut effective = current.to_vec();
    for old in previous {
        if !current
            .iter()
            .any(|new| new.provider_account_id == old.provider_account_id)
        {
            let mut disappeared = old.clone();
            disappeared.accessible = false;
            disappeared.capabilities.clear();
            effective.push(disappeared);
        }
    }
    effective
}

fn credential_context(connection: &BrokerConnection) -> CredentialContext {
    CredentialContext {
        provider: connection.provider.clone(),
        environment: connection.environment,
    }
}

fn health_from_provider_error(
    error: &ProviderError,
    provider: &ProviderId,
    environment: BrokerEnvironment,
    now_unix_ms: i64,
) -> ConnectionHealth {
    let state = match error.kind() {
        ProviderErrorKind::InvalidCredential | ProviderErrorKind::ExpiredOrInactive => {
            ConnectionHealthState::InvalidCredential
        }
        ProviderErrorKind::InsufficientPermission => ConnectionHealthState::InsufficientPermission,
        ProviderErrorKind::AccountAccessChanged => ConnectionHealthState::AccountAccessChanged,
        ProviderErrorKind::WrongEnvironment
        | ProviderErrorKind::ProviderUnavailable
        | ProviderErrorKind::UnsupportedProvider => ConnectionHealthState::ProviderUnavailable,
    };
    ConnectionHealth {
        state,
        checked_at_unix_ms: Some(now_unix_ms),
        provider: provider.clone(),
        environment,
        reason_code: match error.kind() {
            ProviderErrorKind::InvalidCredential => ConnectionHealthReason::InvalidCredential,
            ProviderErrorKind::ExpiredOrInactive => ConnectionHealthReason::ExpiredOrInactive,
            ProviderErrorKind::InsufficientPermission => ConnectionHealthReason::PermissionDenied,
            ProviderErrorKind::WrongEnvironment => ConnectionHealthReason::WrongEnvironment,
            ProviderErrorKind::ProviderUnavailable | ProviderErrorKind::UnsupportedProvider => {
                ConnectionHealthReason::ProviderUnavailable
            }
            ProviderErrorKind::AccountAccessChanged => ConnectionHealthReason::AccountAccessChanged,
        },
        safe_detail: Some(provider_error_detail(error.kind()).to_owned()),
        retryable: matches!(error.kind(), ProviderErrorKind::ProviderUnavailable),
    }
}

fn provider_error_detail(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::InvalidCredential => "provider rejected credential",
        ProviderErrorKind::InsufficientPermission => "provider permission denied",
        ProviderErrorKind::WrongEnvironment => "credential environment mismatch",
        ProviderErrorKind::ProviderUnavailable => "provider unavailable",
        ProviderErrorKind::ExpiredOrInactive => "credential expired or inactive",
        ProviderErrorKind::AccountAccessChanged => "account access changed",
        ProviderErrorKind::UnsupportedProvider => "provider unsupported",
    }
}

struct BoundTarget {
    connection: BrokerConnection,
    target: AccountTarget,
    capabilities: BTreeSet<ConnectionCapability>,
}

fn authorization_rank(mode: ExecutionAuthorizationMode) -> u8 {
    match mode {
        ExecutionAuthorizationMode::Disabled => 0,
        ExecutionAuthorizationMode::ManualAllowed => 1,
        ExecutionAuthorizationMode::AutomatedAllowed => 2,
    }
}

fn execution_purpose_allowed(
    environment: BrokerEnvironment,
    mode: ExecutionAuthorizationMode,
    purpose: ExecutionPurpose,
) -> bool {
    matches!(
        (environment, purpose, mode),
        (
            BrokerEnvironment::Sandbox,
            ExecutionPurpose::SandboxMutation,
            ExecutionAuthorizationMode::ManualAllowed
                | ExecutionAuthorizationMode::AutomatedAllowed
        ) | (
            BrokerEnvironment::Production,
            ExecutionPurpose::ProductionManual,
            ExecutionAuthorizationMode::ManualAllowed
                | ExecutionAuthorizationMode::AutomatedAllowed
        ) | (
            BrokerEnvironment::Production,
            ExecutionPurpose::ProductionAutomated,
            ExecutionAuthorizationMode::AutomatedAllowed
        )
    )
}

fn authorization_name(mode: ExecutionAuthorizationMode) -> &'static str {
    match mode {
        ExecutionAuthorizationMode::Disabled => "DISABLED",
        ExecutionAuthorizationMode::ManualAllowed => "MANUAL_ALLOWED",
        ExecutionAuthorizationMode::AutomatedAllowed => "AUTOMATED_ALLOWED",
    }
}

fn required_order_capability(environment: BrokerEnvironment) -> ConnectionCapability {
    match environment {
        BrokerEnvironment::Production => ConnectionCapability::ProductionOrdersProviderAllowed,
        BrokerEnvironment::Sandbox => ConnectionCapability::SandboxOrders,
    }
}

fn health_name(state: ConnectionHealthState) -> &'static str {
    match state {
        ConnectionHealthState::Unknown => "UNKNOWN",
        ConnectionHealthState::Validating => "VALIDATING",
        ConnectionHealthState::Healthy => "HEALTHY",
        ConnectionHealthState::InvalidCredential => "INVALID_CREDENTIAL",
        ConnectionHealthState::InsufficientPermission => "INSUFFICIENT_PERMISSION",
        ConnectionHealthState::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
        ConnectionHealthState::AccountAccessChanged => "ACCOUNT_ACCESS_CHANGED",
        ConnectionHealthState::Disabled => "DISABLED",
    }
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("permission denied: {0:?}")]
    PermissionDenied(Permission),
    #[error("connection not found")]
    ConnectionNotFound,
    #[error("account not found")]
    AccountNotFound,
    #[error("account unavailable")]
    AccountUnavailable,
    #[error("connection disabled")]
    ConnectionDisabled,
    #[error("connection unavailable")]
    ConnectionUnavailable,
    #[error("connection must be disabled before deletion")]
    DisableBeforeDelete,
    #[error("account binding does not match explicit target")]
    AccountBindingMismatch,
    #[error("provider credential does not allow requested execution")]
    ProviderDoesNotAllowProductionOrders,
    #[error("vox execution is not authorized for the requested mutation")]
    ExecutionUnauthorized,
    #[error("captured execution authorization revision is stale")]
    StaleExecutionAuthorization,
    #[error("execution authorization revision exhausted")]
    AuthorizationRevisionOverflow,
    #[error("credential rotation compensation failed; credential disabled")]
    CredentialRotationCompensationFailed,
    #[error("provider returned duplicate account identity")]
    DuplicateProviderAccount,
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    SecretStore(#[from] SecretStoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}
