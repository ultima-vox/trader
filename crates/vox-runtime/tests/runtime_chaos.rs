use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use vox_runtime::{
    BrokerAccount, BrokerEvent, BrokerEventClass, BrokerIdentityLinks, BrokerMethod,
    BrokerPortError, BrokerReadPort, BrokerResultClass, CredentialResolution,
    CredentialResolverPort, ExecutionPort, ExecutionResult, ExecutionStreamPort, HealthReadPort,
    InMemoryMetrics, JournalState, MetricLabel, MetricName, MutationKind, MutationRecord,
    OpaqueRef, OperationFact, OperationsPage, OrderFact, PortfolioFact, Provider, ReasonCode,
    ReconciliationConfig, RuntimeConfig, RuntimeCoordinator, RuntimeEnvironment, RuntimeError,
    RuntimeScope, RuntimeState, RuntimeStore, SqliteRuntimeStore, StopFact, StreamKind,
    StreamSignal,
};

#[derive(Clone)]
struct FakeSnapshot {
    accounts: Vec<BrokerAccount>,
    portfolio: PortfolioFact,
    positions: Vec<vox_runtime::PositionFact>,
    active_orders: Vec<OrderFact>,
    order_states: Vec<OrderFact>,
    stops: Vec<StopFact>,
    operations: Vec<OperationFact>,
}

impl FakeSnapshot {
    fn flat(account_id: &str) -> Self {
        Self {
            accounts: vec![BrokerAccount {
                account_id: account_id.into(),
                open: true,
                accessible: true,
            }],
            portfolio: PortfolioFact {
                account_id: account_id.into(),
                currencies: BTreeMap::new(),
                broker_observed_at_unix_ms: Some(1),
            },
            positions: Vec::new(),
            active_orders: Vec::new(),
            order_states: Vec::new(),
            stops: Vec::new(),
            operations: Vec::new(),
        }
    }
}

struct FakeBroker {
    snapshot: Mutex<FakeSnapshot>,
    failures: Mutex<BTreeMap<BrokerMethod, VecDeque<BrokerPortError>>>,
    calls: Mutex<BTreeMap<BrokerMethod, u64>>,
}

impl FakeBroker {
    fn new(snapshot: FakeSnapshot) -> Self {
        Self {
            snapshot: Mutex::new(snapshot),
            failures: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(BTreeMap::new()),
        }
    }

    fn fail_once(&self, method: BrokerMethod, error: BrokerPortError) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.entry(method).or_default().push_back(error);
        }
    }

    fn call_count(&self, method: BrokerMethod) -> u64 {
        self.calls
            .lock()
            .ok()
            .and_then(|calls| calls.get(&method).copied())
            .unwrap_or(0)
    }

    fn before(&self, method: BrokerMethod) -> Result<(), BrokerPortError> {
        if let Ok(mut calls) = self.calls.lock() {
            *calls.entry(method).or_default() += 1;
        }
        self.failures
            .lock()
            .ok()
            .and_then(|mut failures| failures.get_mut(&method).and_then(VecDeque::pop_front))
            .map_or(Ok(()), Err)
    }

    fn snapshot(&self) -> FakeSnapshot {
        self.snapshot
            .lock()
            .map_or_else(|error| error.into_inner().clone(), |value| value.clone())
    }
}

#[async_trait]
impl BrokerReadPort for FakeBroker {
    async fn accounts(&self, _: &RuntimeScope) -> Result<Vec<BrokerAccount>, BrokerPortError> {
        self.before(BrokerMethod::GetAccounts)?;
        Ok(self.snapshot().accounts)
    }

    async fn portfolio(&self, _: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError> {
        self.before(BrokerMethod::GetPortfolio)?;
        Ok(self.snapshot().portfolio)
    }

    async fn positions(
        &self,
        _: &RuntimeScope,
    ) -> Result<Vec<vox_runtime::PositionFact>, BrokerPortError> {
        self.before(BrokerMethod::GetPositions)?;
        Ok(self.snapshot().positions)
    }

    async fn active_orders(&self, _: &RuntimeScope) -> Result<Vec<OrderFact>, BrokerPortError> {
        self.before(BrokerMethod::GetOrders)?;
        Ok(self.snapshot().active_orders)
    }

    async fn stop_orders(
        &self,
        _: &RuntimeScope,
        _: i64,
    ) -> Result<Vec<StopFact>, BrokerPortError> {
        self.before(BrokerMethod::GetStopOrders)?;
        Ok(self.snapshot().stops)
    }

    async fn order_state(
        &self,
        _: &RuntimeScope,
        broker_order_id: Option<&str>,
        logical_request_id: Option<&str>,
    ) -> Result<Option<OrderFact>, BrokerPortError> {
        self.before(BrokerMethod::GetOrderState)?;
        Ok(self.snapshot().order_states.into_iter().find(|order| {
            broker_order_id == Some(order.broker_order_id.as_str())
                || logical_request_id == order.logical_request_id.as_deref()
        }))
    }

    async fn operations_page(
        &self,
        _: &RuntimeScope,
        cursor: Option<&str>,
        _: i64,
        limit: u16,
    ) -> Result<OperationsPage, BrokerPortError> {
        self.before(BrokerMethod::GetOperationsByCursor)?;
        assert_eq!(limit, 1_000);
        Ok(if cursor.is_none() {
            OperationsPage {
                items: self.snapshot().operations,
                next_cursor: None,
            }
        } else {
            OperationsPage {
                items: Vec::new(),
                next_cursor: None,
            }
        })
    }
}

struct FakeExecution {
    results: Mutex<VecDeque<Result<ExecutionResult, BrokerPortError>>>,
    calls: AtomicU64,
}

impl FakeExecution {
    fn new(results: impl IntoIterator<Item = Result<ExecutionResult, BrokerPortError>>) -> Self {
        Self {
            results: Mutex::new(results.into_iter().collect()),
            calls: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl ExecutionPort for FakeExecution {
    async fn dispatch_once(
        &self,
        _: &RuntimeScope,
        _: &MutationRecord,
    ) -> Result<ExecutionResult, BrokerPortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .ok()
            .and_then(|mut results| results.pop_front())
            .unwrap_or_else(|| {
                Ok(ExecutionResult::Rejected {
                    broker_evidence_ref: "fake:no-result".into(),
                })
            })
    }
}

#[derive(Default)]
struct FakeStreams {
    sender: Mutex<Option<mpsc::Sender<StreamSignal>>>,
    connects: AtomicU64,
    disconnects: AtomicU64,
}

impl FakeStreams {
    async fn send(&self, signal: StreamSignal) -> Result<(), Box<dyn std::error::Error>> {
        let sender = self
            .sender
            .lock()
            .map_err(|error| error.to_string())?
            .clone()
            .ok_or("stream sender missing")?;
        sender.send(signal).await?;
        Ok(())
    }
}

#[async_trait]
impl ExecutionStreamPort for FakeStreams {
    async fn connect(
        &self,
        _: &RuntimeScope,
        _: u64,
        output: mpsc::Sender<StreamSignal>,
    ) -> Result<(), BrokerPortError> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut sender) = self.sender.lock() {
            *sender = Some(output.clone());
        }
        output
            .send(StreamSignal::Connected(StreamKind::OrderState))
            .await
            .map_err(|error| permanent("OrdersStreamService", "OrderStateStream", error))?;
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), BrokerPortError> {
        self.disconnects.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeCredential {
    valid: AtomicBool,
    execution_authorized: AtomicBool,
}

impl FakeCredential {
    fn accepted(execution_authorized: bool) -> Self {
        Self {
            valid: AtomicBool::new(true),
            execution_authorized: AtomicBool::new(execution_authorized),
        }
    }
}

#[async_trait]
impl CredentialResolverPort for FakeCredential {
    async fn resolve(&self, _: &RuntimeScope) -> Result<CredentialResolution, BrokerPortError> {
        if self.valid.load(Ordering::SeqCst) {
            Ok(CredentialResolution {
                execution_authorized: self.execution_authorized.load(Ordering::SeqCst),
            })
        } else {
            Err(BrokerPortError {
                service: "CredentialResolver",
                method: "resolve",
                class: BrokerResultClass::Credential,
                message: "credential rejected".into(),
                retry_after: None,
            })
        }
    }
}

type TestCoordinator = RuntimeCoordinator<
    FakeBroker,
    FakeExecution,
    FakeStreams,
    FakeCredential,
    SqliteRuntimeStore,
    InMemoryMetrics,
>;

struct Harness {
    path: PathBuf,
    scope: RuntimeScope,
    store: SqliteRuntimeStore,
    broker: Arc<FakeBroker>,
    execution: Arc<FakeExecution>,
    streams: Arc<FakeStreams>,
    credentials: Arc<FakeCredential>,
    metrics: Arc<InMemoryMetrics>,
    coordinator: Arc<TestCoordinator>,
}

impl Harness {
    fn new(
        snapshot: FakeSnapshot,
        execution_results: impl IntoIterator<Item = Result<ExecutionResult, BrokerPortError>>,
        execution_authorized: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let path = runtime_path("chaos");
        let scope = scope()?;
        let store = SqliteRuntimeStore::open(&path)?;
        let broker = Arc::new(FakeBroker::new(snapshot));
        let execution = Arc::new(FakeExecution::new(execution_results));
        let streams = Arc::new(FakeStreams::default());
        let credentials = Arc::new(FakeCredential::accepted(execution_authorized));
        let metrics = Arc::new(InMemoryMetrics::default());
        let coordinator = RuntimeCoordinator::new(
            scope.clone(),
            store.clone(),
            broker.clone(),
            execution.clone(),
            streams.clone(),
            credentials.clone(),
            metrics.clone(),
            ReconciliationConfig {
                max_safe_read_attempts: 3,
                initial_backoff: Duration::from_millis(1),
                maximum_backoff: Duration::from_millis(5),
            },
            RuntimeConfig {
                shutdown_timeout: Duration::from_secs(1),
            },
        );
        Ok(Self {
            path,
            scope,
            store,
            broker,
            execution,
            streams,
            credentials,
            metrics,
            coordinator,
        })
    }

    fn cleanup(self) {
        let path = self.path.clone();
        drop(self);
        cleanup_path(&path);
    }
}

#[tokio::test]
async fn clean_startup_existing_position_and_read_only_authorization_are_safe()
-> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = FakeSnapshot::flat("account-1");
    snapshot.positions.push(vox_runtime::PositionFact {
        account_id: "account-1".into(),
        instrument_uid: "instrument-1".into(),
        quantity_units: 10,
        broker_observed_at_unix_ms: Some(1),
    });
    let harness = Harness::new(snapshot, [], false)?;
    let report = harness.coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Ready, "{report:?}");
    let health = harness.coordinator.health().await;
    assert_eq!(health.state, RuntimeState::Ready);
    assert!(!health.new_exposure_allowed);
    assert!(matches!(
        harness
            .coordinator
            .dispatch(
                "request-blocked",
                MutationKind::PostOrder,
                "quantity=1",
                "correlation"
            )
            .await,
        Err(RuntimeError::ExecutionGateClosed)
    ));
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn acknowledged_mutation_is_durable_and_duplicate_identity_is_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let links = BrokerIdentityLinks {
        logical_request_id: "request-1".into(),
        broker_order_id: Some("broker-1".into()),
        ..BrokerIdentityLinks::default()
    };
    let harness = Harness::new(
        FakeSnapshot::flat("account-1"),
        [Ok(ExecutionResult::Acknowledged {
            broker_evidence_ref: "broker:accepted".into(),
            links,
        })],
        true,
    )?;
    harness.coordinator.start().await?;
    let receipt = harness
        .coordinator
        .dispatch(
            "request-1",
            MutationKind::PostOrder,
            "quantity=1; account=present",
            "correlation-1",
        )
        .await?;
    assert_eq!(receipt.state, JournalState::Acknowledged);
    assert!(matches!(
        harness
            .coordinator
            .dispatch(
                "request-1",
                MutationKind::PostOrder,
                "quantity=2",
                "correlation-2"
            )
            .await,
        Err(RuntimeError::Store(
            vox_runtime::StoreError::DuplicateMutation
        ))
    ));
    assert_eq!(harness.execution.calls.load(Ordering::SeqCst), 1);
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn unknown_survives_restart_resolves_from_direct_readback_without_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let harness = Harness::new(
        FakeSnapshot::flat("account-1"),
        [Ok(ExecutionResult::UnknownAfterDispatch {
            broker_evidence_ref: Some("tracking:ambiguous".into()),
        })],
        true,
    )?;
    harness.coordinator.start().await?;
    assert!(matches!(
        harness
            .coordinator
            .dispatch(
                "request-unknown",
                MutationKind::PostOrderAsync,
                "quantity=1",
                "correlation-unknown"
            )
            .await,
        Err(RuntimeError::UnknownAfterDispatch(id)) if id == "request-unknown"
    ));
    assert_eq!(harness.execution.calls.load(Ordering::SeqCst), 1);
    harness.coordinator.shutdown().await?;

    let path = harness.path.clone();
    let scope = harness.scope.clone();
    drop(harness.coordinator);
    drop(harness.store);
    let store = SqliteRuntimeStore::open(&path)?;
    let broker = Arc::new(FakeBroker::new({
        let mut snapshot = FakeSnapshot::flat("account-1");
        let order = OrderFact {
            account_id: "account-1".into(),
            broker_order_id: "broker-recovered".into(),
            logical_request_id: Some("request-unknown".into()),
            instrument_uid: "instrument-1".into(),
            active: true,
            terminal: false,
        };
        snapshot.active_orders.push(order.clone());
        snapshot.order_states.push(order);
        snapshot
    }));
    let execution = Arc::new(FakeExecution::new([]));
    let streams = Arc::new(FakeStreams::default());
    let credentials = Arc::new(FakeCredential::accepted(true));
    let metrics = Arc::new(InMemoryMetrics::default());
    let restarted = RuntimeCoordinator::new(
        scope.clone(),
        store.clone(),
        broker,
        execution.clone(),
        streams,
        credentials,
        metrics,
        ReconciliationConfig::default(),
        RuntimeConfig::default(),
    );
    let report = restarted.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Ready);
    assert!(store.unresolved_mutations(&scope.key())?.is_empty());
    assert_eq!(
        store.all_identity_links(&scope.key())?[0]
            .broker_order_id
            .as_deref(),
        Some("broker-recovered")
    );
    assert_eq!(execution.calls.load(Ordering::SeqCst), 0);
    restarted.shutdown().await?;
    drop(restarted);
    cleanup_path(&path);
    Ok(())
}

#[tokio::test]
async fn cancel_absence_never_fabricates_rejection_or_success()
-> Result<(), Box<dyn std::error::Error>> {
    let path = runtime_path("cancel-absence");
    let scope = scope()?;
    {
        let store = SqliteRuntimeStore::open(&path)?;
        let epoch = store.acquire_ownership(&scope, "seed", 1)?;
        seed_unknown(&store, &scope, epoch, "cancel-1", MutationKind::CancelOrder)?;
    }
    let store = SqliteRuntimeStore::open(&path)?;
    let broker = Arc::new(FakeBroker::new(FakeSnapshot::flat("account-1")));
    let execution = Arc::new(FakeExecution::new([]));
    let coordinator = RuntimeCoordinator::new(
        scope,
        store,
        broker,
        execution.clone(),
        Arc::new(FakeStreams::default()),
        Arc::new(FakeCredential::accepted(true)),
        Arc::new(InMemoryMetrics::default()),
        ReconciliationConfig::default(),
        RuntimeConfig::default(),
    );
    let report = coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Halted);
    assert_eq!(report.unresolved_logical_request_ids, ["cancel-1"]);
    assert_eq!(execution.calls.load(Ordering::SeqCst), 0);
    coordinator.shutdown().await?;
    drop(coordinator);
    cleanup_path(&path);
    Ok(())
}

#[tokio::test]
async fn manual_order_and_orphan_stop_are_preserved_and_halt_runtime()
-> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = FakeSnapshot::flat("account-1");
    snapshot.positions.push(vox_runtime::PositionFact {
        account_id: "account-1".into(),
        instrument_uid: "instrument-1".into(),
        quantity_units: 1,
        broker_observed_at_unix_ms: Some(1),
    });
    snapshot.active_orders.push(OrderFact {
        account_id: "account-1".into(),
        broker_order_id: "manual-order".into(),
        logical_request_id: None,
        instrument_uid: "instrument-1".into(),
        active: true,
        terminal: false,
    });
    snapshot.stops.push(StopFact {
        account_id: "account-1".into(),
        broker_stop_order_id: "manual-stop".into(),
        logical_request_id: None,
        instrument_uid: "instrument-1".into(),
        active: true,
        terminal: false,
    });
    let harness = Harness::new(snapshot, [], true)?;
    let report = harness.coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Halted);
    assert!(
        report
            .discrepancies
            .iter()
            .any(|detail| detail.contains("manual-order"))
    );
    assert!(
        report
            .discrepancies
            .iter()
            .any(|detail| detail.contains("manual-stop"))
    );
    assert_eq!(harness.execution.calls.load(Ordering::SeqCst), 0);
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn stream_gap_closes_gate_reconnects_and_reconciles() -> Result<(), Box<dyn std::error::Error>>
{
    let harness = Harness::new(FakeSnapshot::flat("account-1"), [], true)?;
    harness.coordinator.start().await?;
    let reads_before = harness.broker.call_count(BrokerMethod::GetAccounts);
    harness
        .streams
        .send(StreamSignal::Gap {
            stream: StreamKind::OrderState,
            reason: "forced deterministic gap".into(),
        })
        .await?;
    for _ in 0..100 {
        if harness.broker.call_count(BrokerMethod::GetAccounts) > reads_before
            && harness.coordinator.health().await.state == RuntimeState::Ready
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(harness.broker.call_count(BrokerMethod::GetAccounts) > reads_before);
    assert!(harness.streams.connects.load(Ordering::SeqCst) >= 2);
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn duplicate_and_stale_epoch_events_are_idempotent() -> Result<(), Box<dyn std::error::Error>>
{
    let harness = Harness::new(FakeSnapshot::flat("account-1"), [], true)?;
    harness.coordinator.start().await?;
    let epoch = harness.coordinator.health().await.runtime_epoch;
    let event = BrokerEvent {
        account_id: "account-1".into(),
        event_class: BrokerEventClass::Fill,
        stable_event_id: "fill-1".into(),
        broker_order_id: Some("broker-1".into()),
        broker_stop_order_id: None,
        logical_request_id: None,
        runtime_epoch: epoch,
    };
    harness
        .streams
        .send(StreamSignal::Event(event.clone()))
        .await?;
    harness.streams.send(StreamSignal::Event(event)).await?;
    harness
        .streams
        .send(StreamSignal::Event(BrokerEvent {
            stable_event_id: "fill-old".into(),
            runtime_epoch: epoch.saturating_sub(1),
            account_id: "account-1".into(),
            event_class: BrokerEventClass::Fill,
            broker_order_id: Some("broker-old".into()),
            broker_stop_order_id: None,
            logical_request_id: None,
        }))
        .await?;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let metrics = harness.metrics.snapshot();
    assert!(metrics.iter().any(|((name, labels), value)| {
        *name == MetricName::EventDeduplicatedTotal
            && labels.contains(&MetricLabel::EventClass(BrokerEventClass::Fill))
            && value.counter == 1
    }));
    assert!(metrics.iter().any(|((name, labels), value)| {
        *name == MetricName::StreamDroppedTotal
            && labels.contains(&MetricLabel::Reason(ReasonCode::StaleEpoch))
            && value.counter == 1
    }));
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn unfamiliar_stream_order_closes_gate_even_when_unary_readback_lags()
-> Result<(), Box<dyn std::error::Error>> {
    let harness = Harness::new(FakeSnapshot::flat("account-1"), [], true)?;
    harness.coordinator.start().await?;
    let epoch = harness.coordinator.health().await.runtime_epoch;
    harness
        .streams
        .send(StreamSignal::Event(BrokerEvent {
            account_id: "account-1".into(),
            event_class: BrokerEventClass::Order,
            stable_event_id: "external-order-new".into(),
            broker_order_id: Some("external-broker-order".into()),
            broker_stop_order_id: None,
            logical_request_id: None,
            runtime_epoch: epoch,
        }))
        .await?;
    for _ in 0..100 {
        if harness.coordinator.health().await.state == RuntimeState::Halted {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let health = harness.coordinator.health().await;
    assert_eq!(health.state, RuntimeState::Halted);
    assert_eq!(health.reason_code, ReasonCode::BrokerOrderConflict);
    assert!(!health.new_exposure_allowed);
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn safe_reads_retry_429_and_transient_failures_with_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let harness = Harness::new(FakeSnapshot::flat("account-1"), [], true)?;
    harness.broker.fail_once(
        BrokerMethod::GetAccounts,
        BrokerPortError {
            service: "UsersService",
            method: "GetAccounts",
            class: BrokerResultClass::RateLimited,
            message: "429".into(),
            retry_after: Some(Duration::from_millis(1)),
        },
    );
    harness.broker.fail_once(
        BrokerMethod::GetPortfolio,
        BrokerPortError {
            service: "OperationsService",
            method: "GetPortfolio",
            class: BrokerResultClass::Transient,
            message: "UNAVAILABLE".into(),
            retry_after: None,
        },
    );
    let report = harness.coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Ready);
    assert!(harness.broker.call_count(BrokerMethod::GetAccounts) >= 3);
    assert!(harness.broker.call_count(BrokerMethod::GetPortfolio) >= 3);
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn invalid_credential_and_inaccessible_account_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let credential_harness = Harness::new(FakeSnapshot::flat("account-1"), [], true)?;
    credential_harness
        .credentials
        .valid
        .store(false, Ordering::SeqCst);
    assert!(matches!(
        credential_harness.coordinator.start().await,
        Err(RuntimeError::Broker(BrokerPortError {
            class: BrokerResultClass::Credential,
            ..
        }))
    ));
    assert_eq!(
        credential_harness.coordinator.health().await.state,
        RuntimeState::Halted
    );
    credential_harness.coordinator.shutdown().await?;
    credential_harness.cleanup();

    let mut snapshot = FakeSnapshot::flat("account-1");
    snapshot.accounts[0].accessible = false;
    let account_harness = Harness::new(snapshot, [], true)?;
    let report = account_harness.coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Halted);
    assert_eq!(report.reason_code, ReasonCode::AccountUnavailable);
    account_harness.coordinator.shutdown().await?;
    account_harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn position_conflict_and_stream_overflow_force_reconciliation()
-> Result<(), Box<dyn std::error::Error>> {
    let links = BrokerIdentityLinks {
        logical_request_id: "position-command".into(),
        broker_order_id: Some("broker-position".into()),
        ..BrokerIdentityLinks::default()
    };
    let harness = Harness::new(
        FakeSnapshot::flat("account-1"),
        [Ok(ExecutionResult::Acknowledged {
            broker_evidence_ref: "accepted".into(),
            links,
        })],
        true,
    )?;
    harness.coordinator.start().await?;
    harness
        .coordinator
        .dispatch(
            "position-command",
            MutationKind::PostOrder,
            "quantity=1",
            "position-correlation",
        )
        .await?;
    let epoch = harness.coordinator.health().await.runtime_epoch;
    harness
        .store
        .set_expected_position(&vox_runtime::DerivedPositionExpectation {
            scope_key: harness.scope.key(),
            instrument_uid: "instrument-1".into(),
            expected_quantity_units: 1,
            based_on_logical_request_id: "position-command".into(),
            runtime_epoch: epoch,
            updated_at_unix_ms: 1,
        })?;
    let report = harness
        .coordinator
        .report_stream_overflow(StreamKind::Positions)
        .await?;
    assert_eq!(report.resulting_state, RuntimeState::Halted);
    assert_eq!(report.reason_code, ReasonCode::BrokerPositionConflict);
    harness.coordinator.shutdown().await?;
    harness.cleanup();
    Ok(())
}

#[tokio::test]
async fn graceful_shutdown_preserves_ambiguous_mutation_and_releases_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    let harness = Harness::new(
        FakeSnapshot::flat("account-1"),
        [Ok(ExecutionResult::UnknownAfterDispatch {
            broker_evidence_ref: None,
        })],
        true,
    )?;
    harness.coordinator.start().await?;
    let _ = harness
        .coordinator
        .dispatch(
            "shutdown-unknown",
            MutationKind::PostOrder,
            "quantity=1",
            "shutdown-correlation",
        )
        .await;
    harness.coordinator.shutdown().await?;
    assert_eq!(
        harness.coordinator.health().await.state,
        RuntimeState::Stopped
    );
    let path = harness.path.clone();
    let scope_key = harness.scope.key();
    drop(harness.coordinator);
    drop(harness.store);
    let reopened = SqliteRuntimeStore::open(&path)?;
    assert_eq!(reopened.unresolved_mutations(&scope_key)?.len(), 1);
    drop(reopened);
    cleanup_path(&path);
    Ok(())
}

#[tokio::test]
async fn restart_with_vox_owned_open_order_and_stop_converges_without_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let path = runtime_path("owned-open-state");
    let scope = scope()?;
    {
        let store = SqliteRuntimeStore::open(&path)?;
        let epoch = store.acquire_ownership(&scope, "seed", 1)?;
        seed_acknowledged(
            &store,
            &scope,
            epoch,
            "owned-order",
            MutationKind::PostOrder,
            BrokerIdentityLinks {
                logical_request_id: "owned-order".into(),
                broker_order_id: Some("broker-owned-order".into()),
                ..BrokerIdentityLinks::default()
            },
        )?;
        seed_acknowledged(
            &store,
            &scope,
            epoch,
            "owned-stop",
            MutationKind::PostStopOrder,
            BrokerIdentityLinks {
                logical_request_id: "owned-stop".into(),
                broker_stop_order_id: Some("broker-owned-stop".into()),
                ..BrokerIdentityLinks::default()
            },
        )?;
    }
    let mut snapshot = FakeSnapshot::flat("account-1");
    snapshot.positions.push(vox_runtime::PositionFact {
        account_id: "account-1".into(),
        instrument_uid: "instrument-1".into(),
        quantity_units: 1,
        broker_observed_at_unix_ms: Some(1),
    });
    snapshot.active_orders.push(OrderFact {
        account_id: "account-1".into(),
        broker_order_id: "broker-owned-order".into(),
        logical_request_id: Some("owned-order".into()),
        instrument_uid: "instrument-1".into(),
        active: true,
        terminal: false,
    });
    snapshot.stops.push(StopFact {
        account_id: "account-1".into(),
        broker_stop_order_id: "broker-owned-stop".into(),
        logical_request_id: Some("owned-stop".into()),
        instrument_uid: "instrument-1".into(),
        active: true,
        terminal: false,
    });
    let store = SqliteRuntimeStore::open(&path)?;
    let execution = Arc::new(FakeExecution::new([]));
    let coordinator = RuntimeCoordinator::new(
        scope,
        store,
        Arc::new(FakeBroker::new(snapshot)),
        execution.clone(),
        Arc::new(FakeStreams::default()),
        Arc::new(FakeCredential::accepted(true)),
        Arc::new(InMemoryMetrics::default()),
        ReconciliationConfig::default(),
        RuntimeConfig::default(),
    );
    let report = coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Ready, "{report:?}");
    assert_eq!(report.active_order_count, 1);
    assert_eq!(report.active_stop_count, 1);
    assert_eq!(execution.calls.load(Ordering::SeqCst), 0);
    coordinator.shutdown().await?;
    drop(coordinator);
    cleanup_path(&path);
    Ok(())
}

#[tokio::test]
async fn protection_legs_resolve_independently_and_partial_plan_stays_halted()
-> Result<(), Box<dyn std::error::Error>> {
    let path = runtime_path("protection-legs");
    let scope = scope()?;
    {
        let store = SqliteRuntimeStore::open(&path)?;
        let epoch = store.acquire_ownership(&scope, "seed", 1)?;
        seed_unknown(
            &store,
            &scope,
            epoch,
            "stop-leg",
            MutationKind::ProtectionLeg,
        )?;
        seed_unknown(
            &store,
            &scope,
            epoch,
            "take-profit-leg",
            MutationKind::ProtectionLeg,
        )?;
    }
    let mut snapshot = FakeSnapshot::flat("account-1");
    snapshot.stops.push(StopFact {
        account_id: "account-1".into(),
        broker_stop_order_id: "broker-stop-leg".into(),
        logical_request_id: Some("stop-leg".into()),
        instrument_uid: "instrument-1".into(),
        active: true,
        terminal: false,
    });
    let store = SqliteRuntimeStore::open(&path)?;
    let coordinator = RuntimeCoordinator::new(
        scope.clone(),
        store.clone(),
        Arc::new(FakeBroker::new(snapshot)),
        Arc::new(FakeExecution::new([])),
        Arc::new(FakeStreams::default()),
        Arc::new(FakeCredential::accepted(true)),
        Arc::new(InMemoryMetrics::default()),
        ReconciliationConfig::default(),
        RuntimeConfig::default(),
    );
    let report = coordinator.start().await?;
    assert_eq!(report.resulting_state, RuntimeState::Halted);
    assert_eq!(report.resolved_logical_request_ids, ["stop-leg"]);
    assert_eq!(report.unresolved_logical_request_ids, ["take-profit-leg"]);
    assert_eq!(store.unresolved_mutations(&scope.key())?.len(), 1);
    coordinator.shutdown().await?;
    drop(coordinator);
    cleanup_path(&path);
    Ok(())
}

#[tokio::test]
async fn corrupt_checkpoint_rebuilds_but_corrupt_unknown_evidence_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let path = runtime_path("corruption");
    let scope = scope()?;
    let harness = Harness::new(FakeSnapshot::flat("account-1"), [], true)?;
    let checkpoint_path = harness.path.clone();
    harness.coordinator.start().await?;
    harness.coordinator.shutdown().await?;
    drop(harness.coordinator);
    drop(harness.store);
    {
        let connection = rusqlite::Connection::open(&checkpoint_path)?;
        connection.execute(
            "UPDATE reconciliation_checkpoint SET reconciliation_id=''",
            [],
        )?;
    }
    let store = SqliteRuntimeStore::open(&checkpoint_path)?;
    let coordinator = RuntimeCoordinator::new(
        scope.clone(),
        store,
        Arc::new(FakeBroker::new(FakeSnapshot::flat("account-1"))),
        Arc::new(FakeExecution::new([])),
        Arc::new(FakeStreams::default()),
        Arc::new(FakeCredential::accepted(true)),
        Arc::new(InMemoryMetrics::default()),
        ReconciliationConfig::default(),
        RuntimeConfig::default(),
    );
    assert_eq!(
        coordinator.start().await?.resulting_state,
        RuntimeState::Ready
    );
    coordinator.shutdown().await?;
    drop(coordinator);
    cleanup_path(&checkpoint_path);

    let corrupt_path = path;
    {
        let store = SqliteRuntimeStore::open(&corrupt_path)?;
        let epoch = store.acquire_ownership(&scope, "seed", 1)?;
        seed_unknown(
            &store,
            &scope,
            epoch,
            "corrupt-unknown",
            MutationKind::PostOrder,
        )?;
    }
    {
        let connection = rusqlite::Connection::open(&corrupt_path)?;
        connection.execute(
            "UPDATE mutation_journal SET mutation_kind='not-json' WHERE logical_request_id='corrupt-unknown'",
            [],
        )?;
    }
    let store = SqliteRuntimeStore::open(&corrupt_path)?;
    let coordinator = RuntimeCoordinator::new(
        scope,
        store,
        Arc::new(FakeBroker::new(FakeSnapshot::flat("account-1"))),
        Arc::new(FakeExecution::new([])),
        Arc::new(FakeStreams::default()),
        Arc::new(FakeCredential::accepted(true)),
        Arc::new(InMemoryMetrics::default()),
        ReconciliationConfig::default(),
        RuntimeConfig::default(),
    );
    assert!(matches!(
        coordinator.start().await,
        Err(RuntimeError::Reconciliation(
            vox_runtime::ReconciliationError::Store(_)
        ))
    ));
    assert_eq!(coordinator.health().await.state, RuntimeState::Halted);
    coordinator.shutdown().await?;
    drop(coordinator);
    cleanup_path(&corrupt_path);
    Ok(())
}

fn scope() -> Result<RuntimeScope, vox_runtime::ModelError> {
    RuntimeScope::new(
        Provider::TInvest,
        RuntimeEnvironment::Sandbox,
        "account-1",
        OpaqueRef::new("connection:primary")?,
        OpaqueRef::new("credential:primary")?,
    )
}

fn seed_unknown(
    store: &SqliteRuntimeStore,
    scope: &RuntimeScope,
    epoch: u64,
    id: &str,
    kind: MutationKind,
) -> Result<(), Box<dyn std::error::Error>> {
    let record = MutationRecord::prepared(
        scope,
        id,
        kind,
        "account=present; broker_identity=present",
        format!("correlation-{id}"),
        epoch,
        1,
    )?;
    store.insert_mutation(&record)?;
    store.claim_dispatch_unknown(&scope.key(), id, epoch, 2)?;
    Ok(())
}

fn seed_acknowledged(
    store: &SqliteRuntimeStore,
    scope: &RuntimeScope,
    epoch: u64,
    id: &str,
    kind: MutationKind,
    links: BrokerIdentityLinks,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_unknown(store, scope, epoch, id, kind)?;
    store.upsert_identity_links(&scope.key(), &links, epoch, 3)?;
    store.transition_mutation(
        &scope.key(),
        id,
        &[JournalState::UnknownAfterDispatch],
        JournalState::Acknowledged,
        Some("broker:acknowledged"),
        Some("seeded authoritative acknowledgement"),
        epoch,
        4,
    )?;
    Ok(())
}

fn runtime_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "vox-runtime-{name}-{}.sqlite",
        uuid::Uuid::new_v4()
    ))
}

fn cleanup_path(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("runtime.lock"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

fn permanent(
    service: &'static str,
    method: &'static str,
    error: impl core::fmt::Display,
) -> BrokerPortError {
    BrokerPortError {
        service,
        method,
        class: BrokerResultClass::Permanent,
        message: error.to_string(),
        retry_after: None,
    }
}
