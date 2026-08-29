#![forbid(unsafe_code)]

pub mod model;
pub mod repository;
pub mod secret;
pub mod service;

pub use model::*;
pub use repository::{ConnectionRepository, SqliteConnectionRepository};
pub use secret::{
    CredentialContext, KeyMaterial, KeyProvider, KeyProviderError, SecretBytes, SecretStore,
    SecretStoreError, SqliteSecretStore, StaticKeyProvider,
};
pub use service::{
    BrokerProviderPort, ConnectionService, CreateConnectionRequest, CredentialRotationOutcome,
    ExecutionPurpose, ProviderAccountFact, ProviderDiscovery, ProviderError, ProviderErrorKind,
    ResolvedExecutionAccess, ResolvedReadAccess, SecurityContext, ServiceError,
};
