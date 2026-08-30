#![forbid(unsafe_code)]

pub mod model;
pub mod repository;
pub mod secret;
pub mod service;

pub use model::*;
pub use repository::{ConnectionRepository, RepositoryError, SqliteConnectionRepository};
pub use secret::{
    CredentialContext, KeyMaterial, KeyProvider, KeyProviderError, SecretBytes, SecretStore,
    SecretStoreError, SqliteSecretStore, StaticKeyProvider,
};
pub use service::{
    BrokerCredentialClientFactory, BrokerProviderPort, ConnectionService, CreateConnectionRequest,
    CredentialRotationOutcome, ExecutionAccessGrant, ExecutionPurpose, ProviderAccountFact,
    ProviderDiscovery, ProviderError, ProviderErrorKind, ReadAccessGrant, SecurityContext,
    ServiceError,
};
