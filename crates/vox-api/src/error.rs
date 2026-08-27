//! One versioned error envelope for the whole public API.
//!
//! Status codes follow RFC 9110 semantics: 4xx when the caller must change something, 5xx
//! when the server or a dependency did. Permission is distinct from validation, and
//! conflict, staleness and unresolved-unknown are distinct from a generic failure, because
//! an operator's next action differs in each case.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The class of failure, which decides what the operator can do about it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCategory {
    /// The request itself is malformed or fails a field rule.
    Validation,
    /// The caller is not authenticated.
    Authentication,
    /// The caller is authenticated but not permitted.
    Permission,
    /// The addressed thing does not exist.
    NotFound,
    /// The request conflicts with current state.
    Conflict,
    /// The request belongs to a scope or epoch that has moved on.
    Stale,
    /// The command reached the broker but its outcome is not known yet.
    UnresolvedUnknown,
    /// The capability has no backend owner in this deployment.
    CapabilityUnavailable,
    /// A dependency failed transiently.
    Transient,
    /// Anything else, which is a bug until proven otherwise.
    Internal,
}

impl ErrorCategory {
    /// RFC 9110 status for this category.
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::Validation => StatusCode::BAD_REQUEST,
            Self::Authentication => StatusCode::UNAUTHORIZED,
            Self::Permission => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            // The scope or epoch moved on: the caller must re-read before retrying.
            Self::Stale => StatusCode::CONFLICT,
            // The outcome is genuinely not known yet; this is not a failure to retry blindly.
            Self::UnresolvedUnknown => StatusCode::ACCEPTED,
            Self::CapabilityUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Transient => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Whether an identical retry can succeed without the caller changing anything.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Transient)
    }
}

/// One field-level complaint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct FieldError {
    /// JSON pointer-ish path of the offending field.
    #[schema(example = "quantity_lots")]
    pub field: String,
    pub message: String,
}

/// The public error envelope. Never carries provider payloads, credentials or stack traces.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ApiError {
    /// Stable machine code, screaming snake case.
    #[schema(example = "CAPABILITY_UNAVAILABLE")]
    pub code: String,
    /// Human sentence for the operator.
    pub message: String,
    /// Correlates this response with server logs and with a mutation, when there is one.
    pub correlation_id: String,
    pub category: ErrorCategory,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_errors: Vec<FieldError>,
    /// Safe, typed extra context. Never provider metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    #[must_use]
    pub fn new(
        category: ErrorCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            correlation_id: new_correlation_id(),
            category,
            retryable: category.retryable(),
            field_errors: Vec::new(),
            details: None,
        }
    }

    /// The capability exists in the design but has no backend owner in this deployment.
    #[must_use]
    pub fn capability_unavailable(capability: &str, owner: &str) -> Self {
        let mut error = Self::new(
            ErrorCategory::CapabilityUnavailable,
            "CAPABILITY_UNAVAILABLE",
            format!(
                "{capability} is not available in this deployment: its backend contract is owned by {owner} and has not landed."
            ),
        );
        error.details = Some(serde_json::json!({ "capability": capability, "owner": owner }));
        error
    }

    #[must_use]
    pub fn validation(message: impl Into<String>, field_errors: Vec<FieldError>) -> Self {
        let mut error = Self::new(ErrorCategory::Validation, "VALIDATION_FAILED", message);
        error.field_errors = field_errors;
        error
    }

    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = correlation_id.into();
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.category.status(), Json(self)).into_response()
    }
}

/// A fresh correlation id for a response that has no upstream one.
fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_map_to_distinct_meaningful_statuses() {
        assert_eq!(ErrorCategory::Validation.status(), StatusCode::BAD_REQUEST);
        assert_eq!(ErrorCategory::Permission.status(), StatusCode::FORBIDDEN);
        assert_ne!(
            ErrorCategory::Permission.status(),
            ErrorCategory::Validation.status(),
            "permission must not read as validation"
        );
        assert_eq!(
            ErrorCategory::CapabilityUnavailable.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            ErrorCategory::UnresolvedUnknown.status(),
            StatusCode::ACCEPTED,
            "an unfinished answer is not a failure"
        );
        assert!(!ErrorCategory::Internal.retryable());
        assert!(ErrorCategory::Transient.retryable());
    }

    #[test]
    fn capability_unavailable_names_the_owner_and_never_pretends_to_succeed()
    -> Result<(), serde_json::Error> {
        let error = ApiError::capability_unavailable("RISK_VERDICT", "#21");
        let json = serde_json::to_value(&error)?;
        assert_eq!(json["code"], "CAPABILITY_UNAVAILABLE");
        assert_eq!(json["details"]["owner"], "#21");
        assert_eq!(json["category"], "CAPABILITY_UNAVAILABLE");
        Ok(())
    }
}
