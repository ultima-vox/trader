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
    ProviderAccountFact, ProviderDiscovery, ProviderError, ProviderErrorKind, ResolvedConnection,
    SecurityContext, ServiceError,
};
