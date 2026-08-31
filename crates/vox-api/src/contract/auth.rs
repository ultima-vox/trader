use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Trusted bootstrap credential. Never returned by Vox.
#[derive(Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    #[schema(write_only)]
    pub bootstrap_credential: String,
}

impl core::fmt::Debug for CreateSessionRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CreateSessionRequest")
            .field("bootstrap_credential", &"[REDACTED]")
            .finish()
    }
}

/// Browser-readable anti-CSRF state. Authentication remains in HttpOnly cookie.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct AuthSessionDto {
    pub csrf_token: String,
    pub expires_at_unix_ms: i64,
}
