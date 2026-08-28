//! Application-side live events.
//!
//! This is the Vox projection bus, not a second T-Invest stream client. #8 and #11 own
//! acquisition and runtime; publishers here already hold accepted facts. The WebSocket
//! gateway fans matching events to bounded per-socket queues.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::application::RuntimeQueries;
use crate::contract::market::{OrderBookDto, QuoteDto, TradeTickDto};
use crate::contract::runtime::RuntimeHealthDto;

/// How many application events may wait for a lagging gateway before it is treated as slow.
pub const APPLICATION_EVENT_CAPACITY: usize = 256;

/// How often a runtime-health watcher re-reads the attached `#11` port.
pub const RUNTIME_HEALTH_WATCH_INTERVAL: Duration = Duration::from_millis(200);

/// A fact the application projection just accepted. Not a broker wire message.
#[derive(Clone, Debug, PartialEq)]
pub enum ApplicationEvent {
    RuntimeHealth(RuntimeHealthDto),
    Quote(QuoteDto),
    OrderBook(OrderBookDto),
    Trades {
        instrument_uid: String,
        ticks: Vec<TradeTickDto>,
    },
}

/// Bounded fan-out of [`ApplicationEvent`]. Lagging subscribers are dropped, not buffered.
#[derive(Clone, Debug)]
pub struct ApplicationEventBus {
    tx: broadcast::Sender<ApplicationEvent>,
}

impl Default for ApplicationEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationEventBus {
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(APPLICATION_EVENT_CAPACITY);
        Self { tx }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ApplicationEvent> {
        self.tx.subscribe()
    }

    /// Publishes one event. No live subscriber is not an error: REST still has the store.
    pub fn publish(&self, event: ApplicationEvent) {
        let _ = self.tx.send(event);
    }
}

/// Watches an attached runtime port and publishes **changes** after the first observation.
///
/// The first read is the snapshot baseline. Later diffs become `UPDATE`s on the gateway.
/// This is application-side polling of `#11`, not a broker stream.
pub fn spawn_runtime_health_watch(runtime: Arc<dyn RuntimeQueries>, events: ApplicationEventBus) {
    tokio::spawn(async move {
        let mut last: Option<RuntimeHealthDto> = None;
        let mut tick = tokio::time::interval(RUNTIME_HEALTH_WATCH_INTERVAL);
        loop {
            tick.tick().await;
            let Ok(health) = runtime.health().await else {
                continue;
            };
            match &last {
                Some(previous) if previous == &health => {}
                Some(_) => {
                    last = Some(health.clone());
                    events.publish(ApplicationEvent::RuntimeHealth(health));
                }
                None => last = Some(health),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_late_subscriber_does_not_see_earlier_events() {
        let bus = ApplicationEventBus::new();
        bus.publish(ApplicationEvent::RuntimeHealth(sample_health("first")));
        let mut rx = bus.subscribe();
        bus.publish(ApplicationEvent::RuntimeHealth(sample_health("second")));
        let ApplicationEvent::RuntimeHealth(health) = rx.recv().await.expect("live event") else {
            panic!("expected runtime health");
        };
        assert_eq!(health.reason, "second");
    }

    fn sample_health(reason: &str) -> RuntimeHealthDto {
        use crate::contract::runtime::{ReasonCodeDto, RuntimeStateDto};
        use crate::contract::scope::{BrokerEnvironment, ProviderDto};
        RuntimeHealthDto {
            state: RuntimeStateDto::Ready,
            reason_code: ReasonCodeDto::ReconciliationComplete,
            reason: reason.to_owned(),
            provider: ProviderDto::TInvest,
            environment: BrokerEnvironment::Sandbox,
            account_display: "sandbox account".to_owned(),
            runtime_epoch: 7,
            connected: true,
            last_successful_reconciliation_at_unix_ms: Some(1),
            reconciliation_age_ms: Some(10),
            unresolved_unknown_count: 0,
            open_order_count: 0,
            active_stop_count: 0,
            stream_states: Vec::new(),
            persistence_healthy: true,
            execution_authorized: false,
            new_exposure_allowed: false,
        }
    }
}
