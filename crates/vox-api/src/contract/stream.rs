//! WebSocket envelopes.
//!
//! One versioned gateway carries every live topic. Each event states its schema version,
//! topic, subscription, scope and event time, so a client can discard anything belonging to
//! a scope or epoch it has already left. Queues are bounded: a slow browser is disconnected
//! with a typed status rather than allowed to grow the server's memory.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::runtime::RuntimeHealthDto;
use super::scope::ExecutionScope;

/// Schema version of the stream envelope. Bumped only through a versioned review.
pub const STREAM_SCHEMA_VERSION: u32 = 1;

/// Topics a client may subscribe to. A topic exists only when its read model does.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Topic {
    /// Runtime health and readiness transitions.
    RuntimeHealth,
    /// Position changes for the subscribed scope.
    Positions,
    /// Order identity and liveness changes.
    Orders,
    /// Stop order and protection changes.
    Stops,
    /// Operations and fills.
    Operations,
    /// Currency balances.
    Portfolio,
}

/// What a client may send.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClientMessage {
    Subscribe {
        /// Client-generated id, echoed on every event of this subscription.
        subscription_id: String,
        topic: Topic,
        /// Required for every account-scoped topic.
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<ExecutionScope>,
    },
    Unsubscribe {
        subscription_id: String,
    },
    Ping {
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<String>,
    },
}

/// Why a subscription is in its current status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionStatus {
    /// Accepted; a snapshot follows.
    Active,
    /// Ended at the client's request.
    Cancelled,
    /// Ended by the server because the client could not keep up.
    DroppedSlowConsumer,
    /// The topic has no backing contract in this deployment.
    Unavailable,
}

/// The payload of an event, keyed by topic.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[serde(tag = "topic", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventPayload {
    RuntimeHealth(RuntimeHealthDto),
}

/// What the server sends.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerEvent {
    /// The full current state of a subscription, sent once on acceptance.
    Snapshot {
        schema_version: u32,
        subscription_id: String,
        /// Event time in milliseconds since the Unix epoch, UTC.
        as_of_unix_ms: i64,
        /// Runtime ownership epoch this event belongs to.
        runtime_epoch: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<ExecutionScope>,
        payload: EventPayload,
    },
    /// A change to an already-delivered snapshot.
    Update {
        schema_version: u32,
        subscription_id: String,
        as_of_unix_ms: i64,
        runtime_epoch: u64,
        /// Monotonic per-subscription sequence, so a gap is detectable.
        sequence: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<ExecutionScope>,
        payload: EventPayload,
    },
    /// The lifecycle of a subscription.
    Status {
        schema_version: u32,
        subscription_id: String,
        status: SubscriptionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// A typed failure. Carries no provider internals and no secrets.
    Error {
        schema_version: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        subscription_id: Option<String>,
        code: String,
        message: String,
        correlation_id: String,
    },
    /// Proof of liveness, also the answer to `PING`.
    Heartbeat {
        schema_version: u32,
        server_time_unix_ms: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        nonce: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_are_tagged_and_screaming_snake() -> Result<(), serde_json::Error> {
        let msg = ClientMessage::Subscribe {
            subscription_id: "s-1".to_owned(),
            topic: Topic::RuntimeHealth,
            scope: None,
        };
        let json = serde_json::to_string(&msg)?;
        assert!(json.contains("\"type\":\"SUBSCRIBE\""), "{json}");
        assert!(json.contains("\"topic\":\"RUNTIME_HEALTH\""), "{json}");
        Ok(())
    }

    #[test]
    fn every_server_event_carries_its_schema_version() -> Result<(), serde_json::Error> {
        let event = ServerEvent::Heartbeat {
            schema_version: STREAM_SCHEMA_VERSION,
            server_time_unix_ms: 1,
            nonce: None,
        };
        let json = serde_json::to_string(&event)?;
        assert!(json.contains("\"type\":\"HEARTBEAT\""), "{json}");
        assert!(json.contains("\"schema_version\":1"), "{json}");
        Ok(())
    }
}
