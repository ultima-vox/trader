use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::ports::{MetricLabel, MetricName, MetricsPort};

const MAX_METRIC_SERIES: usize = 256;
const MAX_LABELS_PER_SERIES: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MetricValue {
    pub gauge: Option<f64>,
    pub counter: u64,
    pub observation_count: u64,
    pub observation_sum: f64,
}

#[derive(Default)]
struct MetricsState {
    values: BTreeMap<(MetricName, Vec<MetricLabel>), MetricValue>,
    rejected_updates: u64,
}

#[derive(Default)]
pub struct InMemoryMetrics {
    state: Mutex<MetricsState>,
}

impl InMemoryMetrics {
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<(MetricName, Vec<MetricLabel>), MetricValue> {
        self.state
            .lock()
            .map_or_else(|_| BTreeMap::new(), |state| state.values.clone())
    }

    #[must_use]
    pub fn rejected_updates(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.rejected_updates)
    }

    fn update(
        &self,
        metric: MetricName,
        labels: &[MetricLabel],
        apply: impl FnOnce(&mut MetricValue),
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if labels.len() > MAX_LABELS_PER_SERIES {
            state.rejected_updates = state.rejected_updates.saturating_add(1);
            return;
        }
        let mut labels = labels.to_vec();
        labels.sort_unstable();
        labels.dedup();
        let key = (metric, labels);
        if !state.values.contains_key(&key) && state.values.len() >= MAX_METRIC_SERIES {
            state.rejected_updates = state.rejected_updates.saturating_add(1);
            return;
        }
        apply(state.values.entry(key).or_default());
    }
}

impl MetricsPort for InMemoryMetrics {
    fn set_gauge(&self, metric: MetricName, labels: &[MetricLabel], value: f64) {
        self.update(metric, labels, |entry| entry.gauge = Some(value));
    }

    fn increment(&self, metric: MetricName, labels: &[MetricLabel], amount: u64) {
        self.update(metric, labels, |entry| {
            entry.counter = entry.counter.saturating_add(amount);
        });
    }

    fn observe_seconds(&self, metric: MetricName, labels: &[MetricLabel], value: f64) {
        self.update(metric, labels, |entry| {
            entry.observation_count = entry.observation_count.saturating_add(1);
            entry.observation_sum += value;
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{ReasonCode, StreamKind};

    use super::*;

    #[test]
    fn labels_are_bounded_typed_and_order_independent() {
        let metrics = InMemoryMetrics::default();
        let labels = [
            MetricLabel::Reason(ReasonCode::StreamGap),
            MetricLabel::Stream(StreamKind::Trades),
        ];
        metrics.increment(MetricName::StreamReconnectTotal, &labels, 1);
        metrics.increment(MetricName::StreamReconnectTotal, &[labels[1], labels[0]], 1);
        assert_eq!(metrics.snapshot().len(), 1);
        assert_eq!(
            metrics
                .snapshot()
                .values()
                .next()
                .map(|value| value.counter),
            Some(2)
        );

        metrics.increment(
            MetricName::BrokerRequestsTotal,
            &[labels[0], labels[1], labels[0], labels[1], labels[0]],
            1,
        );
        assert_eq!(metrics.rejected_updates(), 1);
    }
}
