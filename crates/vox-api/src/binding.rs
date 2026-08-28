//! Explicit mapping from canonical Vox account identity to broker account identity.
//!
//! Public `account_id` is not a broker account id. A caller that needs a `RuntimeScope`
//! must resolve an [`AccountBinding`] first. Matching strings are never treated as a
//! binding: only an inserted triple is authoritative.
//!
//! # Connection identity
//!
//! Public `broker_connection_id` is the same opaque application identity as
//! `RuntimeScope.connection_ref`. The only allowed conversion is
//! [`connection_ref_from_broker_connection_id`]. Call sites must not compare or copy
//! those strings ad hoc. This layer never reads credential material; `OpaqueRef`
//! rejects secret-shaped values.

use std::collections::BTreeMap;

use vox_runtime::{ModelError, OpaqueRef};

/// One authoritative account binding. #17 owns persistence of these records; this crate
/// only defines the port and an in-memory resolver for tests and composition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountBinding {
    account_id: String,
    broker_connection_id: String,
    broker_account_id: String,
}

impl AccountBinding {
    pub fn new(
        account_id: impl Into<String>,
        broker_connection_id: impl Into<String>,
        broker_account_id: impl Into<String>,
    ) -> Result<Self, BindingError> {
        let account_id = require_id(account_id.into(), BindingError::EmptyAccount)?;
        let broker_connection_id =
            require_id(broker_connection_id.into(), BindingError::EmptyConnection)?;
        let broker_account_id =
            require_id(broker_account_id.into(), BindingError::EmptyBrokerAccount)?;
        Ok(Self {
            account_id,
            broker_connection_id,
            broker_account_id,
        })
    }

    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    #[must_use]
    pub fn broker_connection_id(&self) -> &str {
        &self.broker_connection_id
    }

    #[must_use]
    pub fn broker_account_id(&self) -> &str {
        &self.broker_account_id
    }
}

/// Why a canonical account cannot be resolved to a broker account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingError {
    EmptyAccount,
    EmptyConnection,
    EmptyBrokerAccount,
    UnknownAccount,
    BoundToOtherConnection {
        account_id: String,
        bound_connection_id: String,
        requested_connection_id: String,
    },
    DuplicateAccount(String),
}

impl core::fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyAccount => formatter.write_str("account_id cannot be empty"),
            Self::EmptyConnection => formatter.write_str("broker_connection_id cannot be empty"),
            Self::EmptyBrokerAccount => formatter.write_str("broker_account_id cannot be empty"),
            Self::UnknownAccount => formatter.write_str("no binding exists for this account_id"),
            Self::BoundToOtherConnection {
                account_id,
                bound_connection_id,
                requested_connection_id,
            } => write!(
                formatter,
                "account {account_id} is bound to {bound_connection_id}, not {requested_connection_id}"
            ),
            Self::DuplicateAccount(account_id) => {
                write!(formatter, "account {account_id} is already bound")
            }
        }
    }
}

impl std::error::Error for BindingError {}

/// Resolves a canonical Vox account on a connection to a broker account identity.
///
/// Implementations must not infer a binding from `account_id == broker_account_id`.
pub trait AccountBindingResolver: Send + Sync {
    fn resolve(
        &self,
        account_id: &str,
        broker_connection_id: &str,
    ) -> Result<AccountBinding, BindingError>;
}

/// In-memory resolver keyed only by canonical `account_id`. Testable without a broker adapter.
#[derive(Clone, Debug, Default)]
pub struct StaticAccountBindingResolver {
    by_account: BTreeMap<String, AccountBinding>,
}

impl StaticAccountBindingResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, binding: AccountBinding) -> Result<(), BindingError> {
        if self.by_account.contains_key(binding.account_id()) {
            return Err(BindingError::DuplicateAccount(
                binding.account_id().to_owned(),
            ));
        }
        self.by_account
            .insert(binding.account_id().to_owned(), binding);
        Ok(())
    }
}

impl AccountBindingResolver for StaticAccountBindingResolver {
    fn resolve(
        &self,
        account_id: &str,
        broker_connection_id: &str,
    ) -> Result<AccountBinding, BindingError> {
        let Some(binding) = self.by_account.get(account_id) else {
            return Err(BindingError::UnknownAccount);
        };
        if binding.broker_connection_id() != broker_connection_id {
            return Err(BindingError::BoundToOtherConnection {
                account_id: account_id.to_owned(),
                bound_connection_id: binding.broker_connection_id().to_owned(),
                requested_connection_id: broker_connection_id.to_owned(),
            });
        }
        Ok(binding.clone())
    }
}

/// Why a public connection identity cannot become a runtime `OpaqueRef`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionIdentityError {
    Invalid(ModelError),
}

impl core::fmt::Display for ConnectionIdentityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "invalid broker_connection_id: {error}"),
        }
    }
}

impl std::error::Error for ConnectionIdentityError {}

impl From<ModelError> for ConnectionIdentityError {
    fn from(value: ModelError) -> Self {
        Self::Invalid(value)
    }
}

/// Adapter rule: public `broker_connection_id` **is** `RuntimeScope.connection_ref`.
///
/// This is the only conversion. It validates through `OpaqueRef` (non-empty, not
/// secret-shaped) and does not invent a second identity.
pub fn connection_ref_from_broker_connection_id(
    broker_connection_id: &str,
) -> Result<OpaqueRef, ConnectionIdentityError> {
    OpaqueRef::new(broker_connection_id).map_err(ConnectionIdentityError::from)
}

/// Inverse of [`connection_ref_from_broker_connection_id`].
#[must_use]
pub fn broker_connection_id_from_connection_ref(connection_ref: &OpaqueRef) -> &str {
    connection_ref.as_str()
}

fn require_id(value: String, empty: BindingError) -> Result<String, BindingError> {
    if value.trim().is_empty() {
        Err(empty)
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_canonical_accounts_map_to_different_broker_accounts() -> Result<(), BindingError> {
        let mut resolver = StaticAccountBindingResolver::new();
        resolver.bind(AccountBinding::new(
            "vox-acct-alpha",
            "connection:primary",
            "broker-111",
        )?)?;
        resolver.bind(AccountBinding::new(
            "vox-acct-beta",
            "connection:primary",
            "broker-222",
        )?)?;
        assert_eq!(
            resolver
                .resolve("vox-acct-alpha", "connection:primary")?
                .broker_account_id(),
            "broker-111"
        );
        assert_eq!(
            resolver
                .resolve("vox-acct-beta", "connection:primary")?
                .broker_account_id(),
            "broker-222"
        );
        Ok(())
    }

    #[test]
    fn same_broker_account_under_two_connections_does_not_collide() -> Result<(), BindingError> {
        let mut resolver = StaticAccountBindingResolver::new();
        resolver.bind(AccountBinding::new(
            "vox-acct-one",
            "connection:one",
            "broker-shared",
        )?)?;
        resolver.bind(AccountBinding::new(
            "vox-acct-two",
            "connection:two",
            "broker-shared",
        )?)?;
        let left = resolver.resolve("vox-acct-one", "connection:one")?;
        let right = resolver.resolve("vox-acct-two", "connection:two")?;
        assert_eq!(left.broker_account_id(), right.broker_account_id());
        assert_ne!(left.account_id(), right.account_id());
        assert_ne!(left.broker_connection_id(), right.broker_connection_id());
        assert_eq!(
            resolver.resolve("vox-acct-one", "connection:two"),
            Err(BindingError::BoundToOtherConnection {
                account_id: "vox-acct-one".to_owned(),
                bound_connection_id: "connection:one".to_owned(),
                requested_connection_id: "connection:two".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn unknown_canonical_account_fails_closed() {
        let resolver = StaticAccountBindingResolver::new();
        assert_eq!(
            resolver.resolve("vox-acct-missing", "connection:primary"),
            Err(BindingError::UnknownAccount)
        );
    }

    #[test]
    fn account_bound_to_another_connection_fails_closed() -> Result<(), BindingError> {
        let mut resolver = StaticAccountBindingResolver::new();
        resolver.bind(AccountBinding::new(
            "vox-acct-1",
            "connection:one",
            "broker-1",
        )?)?;
        assert_eq!(
            resolver.resolve("vox-acct-1", "connection:two"),
            Err(BindingError::BoundToOtherConnection {
                account_id: "vox-acct-1".to_owned(),
                bound_connection_id: "connection:one".to_owned(),
                requested_connection_id: "connection:two".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn canonical_account_id_is_never_silently_reused_as_broker_account_id() {
        let resolver = StaticAccountBindingResolver::new();
        // Looks like a broker account id. Without an explicit binding it must not resolve.
        assert_eq!(
            resolver.resolve("2000000001", "connection:primary"),
            Err(BindingError::UnknownAccount)
        );
    }

    #[test]
    fn connection_mapping_is_explicit_and_deterministic() -> Result<(), ConnectionIdentityError> {
        let id = "connection:primary";
        let first = connection_ref_from_broker_connection_id(id)?;
        let second = connection_ref_from_broker_connection_id(id)?;
        assert_eq!(first, second);
        assert_eq!(first.as_str(), id);
        assert_eq!(broker_connection_id_from_connection_ref(&first), id);
        assert!(connection_ref_from_broker_connection_id("Bearer secret").is_err());
        Ok(())
    }
}
