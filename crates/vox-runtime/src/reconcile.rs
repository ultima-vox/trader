use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::model::{
    BrokerEvent, BrokerEventClass, BrokerIdentityLinks, BrokerSnapshot, JournalState, MutationKind,
    MutationRecord, OrderExecutionStatus, ReasonCode, ReconciliationCheckpoint,
    ReconciliationDisposition, RuntimeScope, RuntimeState, StopExecutionStatus,
};
use crate::ports::{
    BrokerMethod, BrokerPortError, BrokerReadPort, BrokerResultClass, MetricLabel, MetricName,
    MetricsPort, RuntimeStore, StoreError,
};

const OPERATIONS_PAGE_LIMIT: u16 = 1_000;
const MAX_OPERATION_PAGES: usize = 100;
const MAX_UNRESOLVED_MUTATIONS: usize = 256;
const HISTORY_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const HISTORY_OVERLAP_MS: i64 = 5 * 60 * 1_000;
const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const MAX_DEDUPE_EVENTS: u32 = 100_000;
const MAX_AUDIT_EVENTS: u32 = 20_000;

#[derive(Clone, Debug)]
pub struct ReconciliationConfig {
    pub max_safe_read_attempts: u8,
    pub initial_backoff: Duration,
    pub maximum_backoff: Duration,
}

impl Default for ReconciliationConfig {
    fn default() -> Self {
        Self {
            max_safe_read_attempts: 3,
            initial_backoff: Duration::from_millis(250),
            maximum_backoff: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationReport {
    pub reconciliation_id: String,
    pub runtime_epoch: u64,
    pub resulting_state: RuntimeState,
    pub reason_code: ReasonCode,
    pub resolved_logical_request_ids: Vec<String>,
    pub unresolved_logical_request_ids: Vec<String>,
    pub discrepancies: Vec<String>,
    pub position_count: usize,
    pub active_order_count: usize,
    pub active_stop_count: usize,
    pub deduplicated_event_count: u64,
    pub completed_at_unix_ms: i64,
}

pub struct Reconciler<R, S, M> {
    reads: Arc<R>,
    store: S,
    metrics: Arc<M>,
    config: ReconciliationConfig,
}

impl<R, S, M> Reconciler<R, S, M>
where
    R: BrokerReadPort + 'static,
    S: RuntimeStore,
    M: MetricsPort + 'static,
{
    #[must_use]
    pub fn new(reads: Arc<R>, store: S, metrics: Arc<M>, config: ReconciliationConfig) -> Self {
        Self {
            reads,
            store,
            metrics,
            config,
        }
    }

    pub async fn reconcile(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
    ) -> Result<ReconciliationReport, ReconciliationError> {
        let started = Instant::now();
        let scope_key = scope.key();
        let now = now_unix_ms()?;
        let reconciliation_id = uuid::Uuid::new_v4().to_string();
        let checkpoint = store_call(self.store.clone(), {
            let scope_key = scope_key.clone();
            move |store| store.load_checkpoint(&scope_key)
        })
        .await?;
        let checkpoint = match checkpoint {
            Some(checkpoint)
                if checkpoint.scope_key == scope_key
                    && !checkpoint.reconciliation_id.trim().is_empty()
                    && checkpoint.snapshot_observed_at_unix_ms > 0 =>
            {
                Some(checkpoint)
            }
            Some(_) => {
                store_call(self.store.clone(), {
                    let scope_key = scope_key.clone();
                    move |store| store.discard_checkpoint(&scope_key, runtime_epoch)
                })
                .await?;
                None
            }
            None => None,
        };

        let from_unix_ms =
            checkpoint
                .as_ref()
                .map_or(now.saturating_sub(HISTORY_WINDOW_MS), |checkpoint| {
                    checkpoint
                        .snapshot_observed_at_unix_ms
                        .saturating_sub(HISTORY_OVERLAP_MS)
                });

        let accounts = self
            .safe_read(BrokerMethod::GetAccounts, || self.reads.accounts(scope))
            .await?;
        let (mut portfolio, positions, active_orders, stop_orders) = tokio::try_join!(
            self.safe_read(BrokerMethod::GetPortfolio, || self.reads.portfolio(scope)),
            self.safe_read(BrokerMethod::GetPositions, || self.reads.positions(scope)),
            self.safe_read(BrokerMethod::GetOrders, || self.reads.active_orders(scope)),
            self.safe_read(BrokerMethod::GetStopOrders, || {
                self.reads.stop_orders(scope, from_unix_ms)
            }),
        )?;
        let operations = self.operations(scope, from_unix_ms).await?;
        portfolio.cash_balances = positions.cash_balances;
        let snapshot = BrokerSnapshot {
            accounts,
            portfolio,
            positions: positions.instruments,
            active_orders,
            stop_orders,
            operations,
            stream_evidence: Vec::new(),
            observed_at_unix_ms: now,
        };

        let unresolved = store_call(self.store.clone(), {
            let scope_key = scope_key.clone();
            move |store| store.unresolved_mutations(&scope_key)
        })
        .await?;
        if unresolved.len() > MAX_UNRESOLVED_MUTATIONS {
            return Err(ReconciliationError::Safety(
                "unresolved mutation set exceeds bounded reconciliation capacity".into(),
            ));
        }
        let existing_links = store_call(self.store.clone(), {
            let scope_key = scope_key.clone();
            move |store| store.all_identity_links(&scope_key)
        })
        .await?;
        let expectations = store_call(self.store.clone(), {
            let scope_key = scope_key.clone();
            move |store| store.expected_positions(&scope_key)
        })
        .await?;

        let mut resolved_records = Vec::new();
        let mut resolved_links = Vec::new();
        let mut unresolved_ids = Vec::new();
        for mutation in unresolved {
            let logical_request_id = mutation.logical_request_id.clone();
            match self
                .resolve_mutation(
                    scope,
                    runtime_epoch,
                    &snapshot,
                    &existing_links,
                    mutation,
                    now,
                )
                .await?
            {
                Some((record, links)) => {
                    resolved_links.push(links);
                    resolved_records.push(record);
                }
                None => unresolved_ids.push(logical_request_id),
            }
        }

        let mut reconciled_links = existing_links;
        for resolved in &resolved_links {
            if let Some(existing) = reconciled_links
                .iter_mut()
                .find(|existing| existing.logical_request_id == resolved.logical_request_id)
            {
                *existing = resolved.clone();
            } else {
                reconciled_links.push(resolved.clone());
            }
        }
        let mut discrepancies =
            validate_snapshot(scope, &snapshot, &reconciled_links, &expectations);

        if !unresolved_ids.is_empty() {
            discrepancies.push(format!(
                "{} safety-critical mutation(s) remain UNKNOWN_AFTER_DISPATCH",
                unresolved_ids.len()
            ));
        }
        let deduplicated_event_count = self
            .persist_fill_dedupe(scope, runtime_epoch, &snapshot, now)
            .await?;
        let resulting_state = if discrepancies.is_empty() {
            RuntimeState::Ready
        } else {
            RuntimeState::Halted
        };
        let reason_code = discrepancy_reason(&discrepancies, &unresolved_ids);
        let operations_cursor = snapshot
            .operations
            .last()
            .map(|operation| operation.cursor.clone());
        let checkpoint = ReconciliationCheckpoint {
            scope_key: scope_key.clone(),
            reconciliation_id: reconciliation_id.clone(),
            operations_cursor,
            snapshot_observed_at_unix_ms: snapshot.observed_at_unix_ms,
            completed_at_unix_ms: now,
            runtime_epoch,
            accounts_complete: true,
            portfolio_complete: true,
            positions_complete: true,
            orders_complete: true,
            stops_complete: true,
            operations_complete: true,
        };
        store_call(self.store.clone(), {
            let checkpoint = checkpoint.clone();
            let resolved_records = resolved_records.clone();
            let resolved_links = resolved_links.clone();
            move |store| {
                store.commit_reconciliation(
                    &checkpoint,
                    &resolved_records,
                    &resolved_links,
                    if resulting_state == RuntimeState::Ready {
                        RuntimeState::Reconciling
                    } else {
                        resulting_state
                    },
                    reason_code,
                )
            }
        })
        .await?;
        store_call(self.store.clone(), {
            let scope_key = scope_key.clone();
            move |store| {
                store.compact(
                    &scope_key,
                    now.saturating_sub(RETENTION_MS),
                    MAX_DEDUPE_EVENTS,
                    MAX_AUDIT_EVENTS,
                )
            }
        })
        .await?;

        self.metrics.increment(
            MetricName::ReconciliationTotal,
            &[MetricLabel::Reason(reason_code)],
            1,
        );
        self.metrics.observe_seconds(
            MetricName::ReconciliationDurationSeconds,
            &[MetricLabel::Reason(reason_code)],
            started.elapsed().as_secs_f64(),
        );
        self.metrics.set_gauge(
            MetricName::UnresolvedUnknown,
            &[],
            unresolved_ids.len() as f64,
        );

        Ok(ReconciliationReport {
            reconciliation_id,
            runtime_epoch,
            resulting_state,
            reason_code,
            resolved_logical_request_ids: resolved_records
                .into_iter()
                .map(|record| record.logical_request_id)
                .collect(),
            unresolved_logical_request_ids: unresolved_ids,
            discrepancies,
            position_count: snapshot.positions.len(),
            active_order_count: snapshot.active_orders.len(),
            active_stop_count: snapshot
                .stop_orders
                .iter()
                .filter(|stop| stop.status.active())
                .count(),
            deduplicated_event_count,
            completed_at_unix_ms: now,
        })
    }

    async fn operations(
        &self,
        scope: &RuntimeScope,
        from_unix_ms: i64,
    ) -> Result<Vec<crate::model::OperationFact>, ReconciliationError> {
        let mut cursor = None;
        let mut seen_cursors = BTreeSet::new();
        let mut operations = Vec::new();
        for _ in 0..MAX_OPERATION_PAGES {
            let requested_cursor = cursor.clone();
            let page = self
                .safe_read(BrokerMethod::GetOperationsByCursor, || {
                    self.reads.operations_page(
                        scope,
                        requested_cursor.as_deref(),
                        from_unix_ms,
                        OPERATIONS_PAGE_LIMIT,
                    )
                })
                .await?;
            operations.extend(page.items);
            let Some(next_cursor) = page.next_cursor else {
                return Ok(operations);
            };
            if next_cursor.trim().is_empty() || !seen_cursors.insert(next_cursor.clone()) {
                return Err(ReconciliationError::Safety(
                    "operations cursor is empty or cyclic".into(),
                ));
            }
            cursor = Some(next_cursor);
        }
        Err(ReconciliationError::Safety(
            "operations history exceeds bounded page count".into(),
        ))
    }

    async fn resolve_mutation(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
        snapshot: &BrokerSnapshot,
        existing_links: &[BrokerIdentityLinks],
        mut mutation: MutationRecord,
        now: i64,
    ) -> Result<Option<(MutationRecord, BrokerIdentityLinks)>, ReconciliationError> {
        let existing = existing_links
            .iter()
            .find(|links| links.logical_request_id == mutation.logical_request_id)
            .cloned()
            .unwrap_or_else(|| BrokerIdentityLinks {
                logical_request_id: mutation.logical_request_id.clone(),
                ..BrokerIdentityLinks::default()
            });
        let order_kind = matches!(
            mutation.kind,
            MutationKind::PostOrder
                | MutationKind::PostOrderAsync
                | MutationKind::ReplaceOrder
                | MutationKind::CancelOrder
        );
        if order_kind {
            let broker_id = match mutation.kind {
                MutationKind::ReplaceOrder => existing.replacement_broker_order_id.as_deref(),
                _ => existing
                    .replacement_broker_order_id
                    .as_deref()
                    .or(existing.broker_order_id.as_deref()),
            };
            let direct = self
                .safe_read(BrokerMethod::GetOrderState, || {
                    self.reads
                        .order_state(scope, broker_id, Some(&mutation.logical_request_id))
                })
                .await?;
            if let Some(order) = direct
                && order_evidence_resolves(&mutation, &existing, &order)
            {
                let mut links = existing;
                if mutation.kind == MutationKind::ReplaceOrder {
                    links.replacement_broker_order_id = Some(order.broker_order_id.clone());
                } else {
                    links.broker_order_id = Some(order.broker_order_id.clone());
                }
                let Some(disposition) = order_disposition(mutation.kind, order.status) else {
                    return Ok(None);
                };
                reconcile_record(
                    &mut mutation,
                    runtime_epoch,
                    now,
                    disposition,
                    format!("direct GetOrderState broker={}", order.broker_order_id),
                );
                return Ok(Some((mutation, links)));
            }
        }

        if let Some(order) = snapshot
            .active_orders
            .iter()
            .find(|order| order_evidence_resolves(&mutation, &existing, order))
            && !matches!(mutation.kind, MutationKind::CancelOrder)
        {
            let mut links = existing.clone();
            if mutation.kind == MutationKind::ReplaceOrder {
                links.replacement_broker_order_id = Some(order.broker_order_id.clone());
            } else {
                links.broker_order_id = Some(order.broker_order_id.clone());
            }
            let Some(disposition) = order_disposition(mutation.kind, order.status) else {
                return Ok(None);
            };
            reconcile_record(
                &mut mutation,
                runtime_epoch,
                now,
                disposition,
                format!(
                    "authoritative active order broker={}",
                    order.broker_order_id
                ),
            );
            return Ok(Some((mutation, links)));
        }

        if let Some(stop) = snapshot.stop_orders.iter().find(|stop| {
            existing.broker_stop_order_id.as_deref() == Some(&stop.broker_stop_order_id)
        }) && let Some(disposition) = stop_disposition(mutation.kind, stop.status)
        {
            let mut links = existing.clone();
            links.broker_stop_order_id = Some(stop.broker_stop_order_id.clone());
            reconcile_record(
                &mut mutation,
                runtime_epoch,
                now,
                disposition,
                format!(
                    "authoritative GetStopOrders broker={}",
                    stop.broker_stop_order_id
                ),
            );
            return Ok(Some((mutation, links)));
        }

        if let Some(operation) = snapshot
            .operations
            .iter()
            .find(|operation| operation_evidence_resolves(&mutation, &existing, operation))
        {
            let mut links = existing.clone();
            if let Some(operation_id) = &operation.provider_operation_id {
                links.provider_operation_ids.insert(operation_id.clone());
            }
            links
                .broker_fill_ids
                .extend(operation.broker_fill_ids.iter().cloned());
            reconcile_record(
                &mut mutation,
                runtime_epoch,
                now,
                ReconciliationDisposition::OrderAccepted,
                format!("GetOperationsByCursor cursor={}", operation.cursor),
            );
            return Ok(Some((mutation, links)));
        }

        if let Some(event) = snapshot
            .stream_evidence
            .iter()
            .find(|event| stream_evidence_resolves(&mutation, &existing, event))
        {
            reconcile_record(
                &mut mutation,
                runtime_epoch,
                now,
                ReconciliationDisposition::OrderAccepted,
                format!("accepted stream event={}", event.stable_event_id),
            );
            return Ok(Some((mutation, existing)));
        }
        Ok(None)
    }

    async fn persist_fill_dedupe(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
        snapshot: &BrokerSnapshot,
        now: i64,
    ) -> Result<u64, ReconciliationError> {
        let mut duplicates = 0_u64;
        for operation in &snapshot.operations {
            for fill_id in &operation.broker_fill_ids {
                let event = BrokerEvent {
                    account_id: scope.broker_account_id.clone(),
                    event_class: BrokerEventClass::Fill,
                    stable_event_id: fill_id.clone(),
                    broker_order_id: operation.broker_order_id.clone(),
                    broker_stop_order_id: None,
                    logical_request_id: operation.logical_request_id.clone(),
                    execution_state: None,
                    runtime_epoch,
                };
                let inserted = store_call(self.store.clone(), {
                    let scope_key = scope.key();
                    move |store| store.record_broker_event(&scope_key, &event, now)
                })
                .await?;
                if !inserted {
                    duplicates = duplicates.saturating_add(1);
                    self.metrics.increment(
                        MetricName::EventDeduplicatedTotal,
                        &[MetricLabel::EventClass(BrokerEventClass::Fill)],
                        1,
                    );
                }
            }
        }
        Ok(duplicates)
    }

    async fn safe_read<T, F, Fut>(
        &self,
        method: BrokerMethod,
        mut operation: F,
    ) -> Result<T, ReconciliationError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, BrokerPortError>>,
    {
        let attempts = self.config.max_safe_read_attempts.max(1);
        let mut backoff = self.config.initial_backoff;
        for attempt in 1..=attempts {
            let started = Instant::now();
            match operation().await {
                Ok(value) => {
                    self.metrics.increment(
                        MetricName::BrokerRequestsTotal,
                        &[
                            MetricLabel::Service(method.service()),
                            MetricLabel::Method(method),
                            MetricLabel::Result(BrokerResultClass::Success),
                        ],
                        1,
                    );
                    self.metrics.observe_seconds(
                        MetricName::BrokerRequestDurationSeconds,
                        &[
                            MetricLabel::Service(method.service()),
                            MetricLabel::Method(method),
                        ],
                        started.elapsed().as_secs_f64(),
                    );
                    return Ok(value);
                }
                Err(error) => {
                    self.metrics.increment(
                        MetricName::BrokerRequestsTotal,
                        &[
                            MetricLabel::Service(method.service()),
                            MetricLabel::Method(method),
                            MetricLabel::Result(error.class),
                        ],
                        1,
                    );
                    if error.class == BrokerResultClass::RateLimited {
                        self.metrics.increment(
                            MetricName::BrokerRateLimitedTotal,
                            &[MetricLabel::Service(method.service())],
                            1,
                        );
                    }
                    if attempt == attempts || !error.safe_read_retryable() {
                        return Err(ReconciliationError::Broker(error));
                    }
                    let delay = error
                        .retry_after
                        .unwrap_or_else(|| jittered(backoff))
                        .min(self.config.maximum_backoff);
                    tokio::time::sleep(delay).await;
                    backoff = backoff
                        .checked_mul(2)
                        .unwrap_or(self.config.maximum_backoff)
                        .min(self.config.maximum_backoff);
                }
            }
        }
        Err(ReconciliationError::Safety(
            "safe-read retry loop exhausted unexpectedly".into(),
        ))
    }
}

fn validate_snapshot(
    scope: &RuntimeScope,
    snapshot: &BrokerSnapshot,
    links: &[BrokerIdentityLinks],
    expectations: &[crate::model::DerivedPositionExpectation],
) -> Vec<String> {
    let mut discrepancies = Vec::new();
    let matching_accounts = snapshot
        .accounts
        .iter()
        .filter(|account| account.account_id == scope.broker_account_id)
        .collect::<Vec<_>>();
    if matching_accounts.len() != 1
        || !matching_accounts[0].open
        || !matching_accounts[0].accessible
    {
        discrepancies.push("broker account missing, closed or inaccessible".into());
    }
    if snapshot.portfolio.account_id != scope.broker_account_id {
        discrepancies.push("portfolio account identity mismatch".into());
    }
    if snapshot
        .positions
        .iter()
        .any(|position| position.account_id != scope.broker_account_id)
    {
        discrepancies.push("position account identity mismatch".into());
    }
    if snapshot
        .active_orders
        .iter()
        .any(|order| order.account_id != scope.broker_account_id)
    {
        discrepancies.push("order account identity mismatch".into());
    }
    if snapshot
        .stop_orders
        .iter()
        .any(|stop| stop.account_id != scope.broker_account_id)
    {
        discrepancies.push("stop account identity mismatch".into());
    }
    if snapshot
        .operations
        .iter()
        .any(|operation| operation.account_id != scope.broker_account_id)
    {
        discrepancies.push("operation account identity mismatch".into());
    }

    duplicate_identity(
        snapshot
            .active_orders
            .iter()
            .map(|order| order.broker_order_id.as_str()),
        "duplicate active broker order identity",
        &mut discrepancies,
    );
    duplicate_identity(
        snapshot
            .stop_orders
            .iter()
            .map(|stop| stop.broker_stop_order_id.as_str()),
        "duplicate broker stop identity",
        &mut discrepancies,
    );
    duplicate_identity(
        snapshot
            .positions
            .iter()
            .map(|position| position.instrument_uid.as_str()),
        "duplicate broker position identity",
        &mut discrepancies,
    );

    let linked_orders = links
        .iter()
        .flat_map(|link| {
            [
                link.broker_order_id.as_deref(),
                link.replacement_broker_order_id.as_deref(),
            ]
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    for order in snapshot
        .active_orders
        .iter()
        .filter(|order| order.status.active())
    {
        if !linked_orders.contains(order.broker_order_id.as_str()) {
            discrepancies.push(format!(
                "unfamiliar broker order preserved: {}",
                order.broker_order_id
            ));
        }
    }
    let linked_stops = links
        .iter()
        .filter_map(|link| link.broker_stop_order_id.as_deref())
        .collect::<BTreeSet<_>>();
    let positions = snapshot
        .positions
        .iter()
        .map(|position| (position.instrument_uid.as_str(), position.quantity_units))
        .collect::<BTreeMap<_, _>>();
    for stop in snapshot
        .stop_orders
        .iter()
        .filter(|stop| stop.status.active())
    {
        if !linked_stops.contains(stop.broker_stop_order_id.as_str()) {
            discrepancies.push(format!(
                "unfamiliar broker stop preserved: {}",
                stop.broker_stop_order_id
            ));
        }
        if positions
            .get(stop.instrument_uid.as_str())
            .copied()
            .unwrap_or(0)
            == 0
        {
            discrepancies.push(format!(
                "active stop {} has no broker position; preserved for operator review",
                stop.broker_stop_order_id
            ));
        }
    }
    for expectation in expectations {
        let actual = positions
            .get(expectation.instrument_uid.as_str())
            .copied()
            .unwrap_or(0);
        if actual != expectation.expected_quantity_units {
            discrepancies.push(format!(
                "derived local position expectation differs for instrument {}",
                expectation.instrument_uid
            ));
        }
    }
    discrepancies
}

fn duplicate_identity<'a>(
    identities: impl Iterator<Item = &'a str>,
    detail: &str,
    discrepancies: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();
    if identities
        .into_iter()
        .any(|identity| !seen.insert(identity))
    {
        discrepancies.push(detail.into());
    }
}

fn order_evidence_resolves(
    mutation: &MutationRecord,
    links: &BrokerIdentityLinks,
    order: &crate::model::OrderFact,
) -> bool {
    let logical_match = order.logical_request_id.as_deref() == Some(&mutation.logical_request_id);
    let original_match = links.broker_order_id.as_deref() == Some(&order.broker_order_id);
    let replacement_match =
        links.replacement_broker_order_id.as_deref() == Some(&order.broker_order_id);
    match mutation.kind {
        MutationKind::PostOrder | MutationKind::PostOrderAsync => logical_match || original_match,
        MutationKind::ReplaceOrder => logical_match || replacement_match,
        MutationKind::CancelOrder => {
            (original_match || replacement_match)
                && order_disposition(mutation.kind, order.status).is_some()
        }
        _ => false,
    }
}

fn operation_evidence_resolves(
    mutation: &MutationRecord,
    links: &BrokerIdentityLinks,
    operation: &crate::model::OperationFact,
) -> bool {
    matches!(
        mutation.kind,
        MutationKind::PostOrder | MutationKind::PostOrderAsync
    ) && (operation.logical_request_id.as_deref() == Some(&mutation.logical_request_id)
        || operation.broker_order_id.as_deref() == links.broker_order_id.as_deref()
            && operation.broker_order_id.is_some())
}

fn stream_evidence_resolves(
    mutation: &MutationRecord,
    links: &BrokerIdentityLinks,
    event: &crate::model::BrokerEvent,
) -> bool {
    let logical_match = event.logical_request_id.as_deref() == Some(&mutation.logical_request_id);
    match mutation.kind {
        MutationKind::PostOrder | MutationKind::PostOrderAsync => {
            matches!(
                event.event_class,
                BrokerEventClass::Order | BrokerEventClass::Fill
            ) && (logical_match
                || event.broker_order_id.as_deref() == links.broker_order_id.as_deref()
                    && event.broker_order_id.is_some())
        }
        MutationKind::ReplaceOrder => {
            event.event_class == BrokerEventClass::Order
                && (logical_match
                    || event.broker_order_id.as_deref()
                        == links.replacement_broker_order_id.as_deref()
                        && event.broker_order_id.is_some())
        }
        MutationKind::PostStopOrder | MutationKind::ProtectionLeg => {
            event.event_class == BrokerEventClass::Stop
                && (logical_match
                    || event.broker_stop_order_id.as_deref()
                        == links.broker_stop_order_id.as_deref()
                        && event.broker_stop_order_id.is_some())
        }
        MutationKind::CancelOrder | MutationKind::CancelStopOrder => false,
    }
}

fn reconcile_record(
    record: &mut MutationRecord,
    runtime_epoch: u64,
    now: i64,
    disposition: ReconciliationDisposition,
    evidence: String,
) {
    record.state = JournalState::Reconciled;
    record.broker_evidence_ref = Some(evidence);
    record.reconciliation_disposition = Some(disposition);
    record.updated_at_unix_ms = now;
    record.runtime_epoch = runtime_epoch;
}

fn order_disposition(
    mutation_kind: MutationKind,
    status: OrderExecutionStatus,
) -> Option<ReconciliationDisposition> {
    match (mutation_kind, status) {
        (MutationKind::CancelOrder, OrderExecutionStatus::Cancelled) => {
            Some(ReconciliationDisposition::OrderCancelled)
        }
        (MutationKind::CancelOrder, OrderExecutionStatus::Filled) => {
            Some(ReconciliationDisposition::OrderFilled)
        }
        (MutationKind::CancelOrder, OrderExecutionStatus::Rejected) => {
            Some(ReconciliationDisposition::OrderRejected)
        }
        (MutationKind::CancelOrder, _) => None,
        (_, OrderExecutionStatus::New) => Some(ReconciliationDisposition::OrderActiveNew),
        (_, OrderExecutionStatus::PartiallyFilled) => {
            Some(ReconciliationDisposition::OrderActivePartial)
        }
        (_, OrderExecutionStatus::Filled) => Some(ReconciliationDisposition::OrderFilled),
        (_, OrderExecutionStatus::Cancelled) => Some(ReconciliationDisposition::OrderCancelled),
        (_, OrderExecutionStatus::Rejected) => Some(ReconciliationDisposition::OrderRejected),
        (_, OrderExecutionStatus::UnknownProviderStatus(_)) => None,
    }
}

fn stop_disposition(
    mutation_kind: MutationKind,
    status: StopExecutionStatus,
) -> Option<ReconciliationDisposition> {
    match (mutation_kind, status) {
        (MutationKind::CancelStopOrder, StopExecutionStatus::Canceled) => {
            Some(ReconciliationDisposition::StopCanceled)
        }
        (MutationKind::CancelStopOrder, StopExecutionStatus::Executed) => {
            Some(ReconciliationDisposition::StopExecuted)
        }
        (MutationKind::CancelStopOrder, StopExecutionStatus::Expired) => {
            Some(ReconciliationDisposition::StopExpired)
        }
        (MutationKind::CancelStopOrder, _) => None,
        (_, StopExecutionStatus::Active) => Some(ReconciliationDisposition::StopActive),
        (_, StopExecutionStatus::Executed) => Some(ReconciliationDisposition::StopExecuted),
        (_, StopExecutionStatus::Canceled) => Some(ReconciliationDisposition::StopCanceled),
        (_, StopExecutionStatus::Expired) => Some(ReconciliationDisposition::StopExpired),
        (_, StopExecutionStatus::UnknownProviderStatus(_)) => None,
    }
}

fn discrepancy_reason(discrepancies: &[String], unresolved: &[String]) -> ReasonCode {
    if !unresolved.is_empty() {
        ReasonCode::UnknownMutation
    } else if discrepancies
        .iter()
        .any(|value| value.contains("account missing, closed or inaccessible"))
    {
        ReasonCode::AccountUnavailable
    } else if discrepancies.iter().any(|value| value.contains("position")) {
        ReasonCode::BrokerPositionConflict
    } else if discrepancies.iter().any(|value| value.contains("order")) {
        ReasonCode::BrokerOrderConflict
    } else if discrepancies.iter().any(|value| value.contains("stop")) {
        ReasonCode::BrokerStopConflict
    } else if discrepancies.is_empty() {
        ReasonCode::ReconciliationComplete
    } else {
        ReasonCode::ReconciliationIncomplete
    }
}

fn jittered(base: Duration) -> Duration {
    let nanos = base.as_nanos();
    let spread = nanos / 4;
    if spread == 0 {
        return base;
    }
    let byte = u128::from(uuid::Uuid::new_v4().as_bytes()[0]);
    let jitter = spread.saturating_mul(byte) / 255;
    let total = nanos.saturating_add(jitter);
    Duration::from_nanos(u64::try_from(total).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn cancel_order_terminal_outcomes_remain_distinct() {
        assert_eq!(
            order_disposition(MutationKind::CancelOrder, OrderExecutionStatus::Cancelled),
            Some(ReconciliationDisposition::OrderCancelled)
        );
        assert_eq!(
            order_disposition(MutationKind::CancelOrder, OrderExecutionStatus::Filled),
            Some(ReconciliationDisposition::OrderFilled)
        );
        assert_eq!(
            order_disposition(MutationKind::CancelOrder, OrderExecutionStatus::Rejected),
            Some(ReconciliationDisposition::OrderRejected)
        );
        assert_eq!(
            order_disposition(
                MutationKind::CancelOrder,
                OrderExecutionStatus::PartiallyFilled
            ),
            None
        );
        assert_eq!(
            order_disposition(
                MutationKind::CancelOrder,
                OrderExecutionStatus::UnknownProviderStatus(77_777)
            ),
            None
        );
    }

    #[test]
    fn cancel_stop_terminal_outcomes_remain_distinct() {
        assert_eq!(
            stop_disposition(MutationKind::CancelStopOrder, StopExecutionStatus::Canceled),
            Some(ReconciliationDisposition::StopCanceled)
        );
        assert_eq!(
            stop_disposition(MutationKind::CancelStopOrder, StopExecutionStatus::Executed),
            Some(ReconciliationDisposition::StopExecuted)
        );
        assert_eq!(
            stop_disposition(MutationKind::CancelStopOrder, StopExecutionStatus::Expired),
            Some(ReconciliationDisposition::StopExpired)
        );
        assert_eq!(
            stop_disposition(MutationKind::CancelStopOrder, StopExecutionStatus::Active),
            None
        );
        assert_eq!(
            stop_disposition(
                MutationKind::CancelStopOrder,
                StopExecutionStatus::UnknownProviderStatus(88_888)
            ),
            None
        );
    }
}

fn now_unix_ms() -> Result<i64, ReconciliationError> {
    let nanos = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000)
        .map_err(|_| ReconciliationError::Safety("system clock is outside i64 range".into()))
}

async fn store_call<S, T, F>(store: S, operation: F) -> Result<T, ReconciliationError>
where
    S: RuntimeStore,
    T: Send + 'static,
    F: FnOnce(S) -> Result<T, StoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || operation(store))
        .await
        .map_err(|error| ReconciliationError::Store(StoreError::BlockingTask(error.to_string())))?
        .map_err(ReconciliationError::Store)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReconciliationError {
    #[error(transparent)]
    Broker(#[from] BrokerPortError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("reconciliation safety failure: {0}")]
    Safety(String),
}
