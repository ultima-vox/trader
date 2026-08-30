use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore, mpsc};

use crate::model::RuntimeExecutionCommandExt;
use crate::model::{
    EXECUTION_QUEUE_CAPACITY, JournalState, MutationRecord, ReasonCode, RuntimeAuditRecord,
    RuntimeExecutionCommand, RuntimeHealth, RuntimeScope, RuntimeState, STREAM_QUEUE_CAPACITY,
    StateTransition, StreamHealth, StreamKind, StreamState,
};
use crate::policy::RuntimeStateMachine;
use crate::ports::{
    BrokerPortError, CredentialResolverPort, ExecutionPort, ExecutionResult, ExecutionStreamPort,
    HealthReadPort, MetricLabel, MetricName, MetricsPort, RuntimeExecutionPurpose, RuntimeStore,
    StoreError, StreamSignal,
};
use crate::reconcile::{Reconciler, ReconciliationError, ReconciliationReport};

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub shutdown_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            shutdown_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchReceipt {
    pub logical_request_id: String,
    pub state: JournalState,
    pub broker_evidence_ref: Option<String>,
}

pub struct RuntimeCoordinator<R, E, X, C, S, M> {
    scope: RuntimeScope,
    store: S,
    reconciler: Reconciler<R, S, M>,
    execution: Arc<E>,
    streams: Arc<X>,
    credentials: Arc<C>,
    metrics: Arc<M>,
    config: RuntimeConfig,
    owner_id: String,
    epoch: AtomicU64,
    started: AtomicBool,
    connected: AtomicBool,
    execution_authorized: AtomicBool,
    state: Mutex<RuntimeStateMachine>,
    health: RwLock<RuntimeHealth>,
    command_slots: Arc<Semaphore>,
    reconciliation_guard: Mutex<()>,
    reconciliation_dirty: AtomicBool,
    reconciliation_running: AtomicBool,
    coalesced_reconciliation_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stream_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stream_sender: Mutex<Option<mpsc::Sender<StreamSignal>>>,
}

impl<R, E, X, C, S, M> RuntimeCoordinator<R, E, X, C, S, M>
where
    R: crate::ports::BrokerReadPort + 'static,
    E: ExecutionPort + 'static,
    X: ExecutionStreamPort + 'static,
    C: CredentialResolverPort + 'static,
    S: RuntimeStore,
    M: MetricsPort + 'static,
{
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        scope: RuntimeScope,
        store: S,
        reads: Arc<R>,
        execution: Arc<E>,
        streams: Arc<X>,
        credentials: Arc<C>,
        metrics: Arc<M>,
        reconciliation_config: crate::reconcile::ReconciliationConfig,
        config: RuntimeConfig,
    ) -> Arc<Self> {
        let health = RuntimeHealth {
            state: RuntimeState::Starting,
            reason_code: ReasonCode::Startup,
            reason: "runtime created; no broker reconciliation yet".into(),
            provider: scope.provider,
            environment: scope.environment,
            account_display: scope.redacted_account_id(),
            runtime_epoch: 0,
            connected: false,
            last_successful_reconciliation_at_unix_ms: None,
            reconciliation_age_ms: None,
            unresolved_unknown_count: 0,
            open_order_count: 0,
            active_stop_count: 0,
            stream_states: all_stream_health(),
            persistence_healthy: true,
            execution_authorized: false,
            new_exposure_allowed: false,
        };
        Arc::new(Self {
            scope,
            store: store.clone(),
            reconciler: Reconciler::new(reads, store, metrics.clone(), reconciliation_config),
            execution,
            streams,
            credentials,
            metrics,
            config,
            owner_id: uuid::Uuid::new_v4().to_string(),
            epoch: AtomicU64::new(0),
            started: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            execution_authorized: AtomicBool::new(false),
            state: Mutex::new(RuntimeStateMachine::default()),
            health: RwLock::new(health),
            command_slots: Arc::new(Semaphore::new(EXECUTION_QUEUE_CAPACITY)),
            reconciliation_guard: Mutex::new(()),
            reconciliation_dirty: AtomicBool::new(false),
            reconciliation_running: AtomicBool::new(false),
            coalesced_reconciliation_task: Mutex::new(None),
            stream_task: Mutex::new(None),
            stream_sender: Mutex::new(None),
        })
    }

    pub async fn start(self: &Arc<Self>) -> Result<ReconciliationReport, RuntimeError> {
        if self
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(RuntimeError::Safety(
                "runtime coordinator instance may start exactly once".into(),
            ));
        }
        let now = now_unix_ms()?;
        let ownership = store_call(self.store.clone(), {
            let scope = self.scope.clone();
            let owner_id = self.owner_id.clone();
            move |store| store.acquire_ownership(&scope, &owner_id, now)
        })
        .await;
        let epoch = match ownership {
            Ok(epoch) => epoch,
            Err(source) => {
                let reason = if source == StoreError::OwnershipUnavailable {
                    ReasonCode::OwnershipFailure
                } else {
                    ReasonCode::PersistenceFailure
                };
                self.state.lock().await.transition(RuntimeState::Halted)?;
                self.update_health(|health| {
                    health.state = RuntimeState::Halted;
                    health.reason_code = reason;
                    health.reason = source.to_string();
                    health.persistence_healthy = false;
                    health.new_exposure_allowed = false;
                })
                .await;
                self.metrics.set_gauge(MetricName::RuntimeReady, &[], 0.0);
                self.metrics.increment(
                    MetricName::PersistenceErrorsTotal,
                    &[MetricLabel::Operation(
                        crate::ports::StoreOperation::Acquire,
                    )],
                    1,
                );
                return Err(RuntimeError::Store(source));
            }
        };
        self.epoch.store(epoch, Ordering::SeqCst);
        self.metrics
            .increment(MetricName::RuntimeEpochChanges, &[], 1);
        self.update_health(|health| health.runtime_epoch = epoch)
            .await;
        self.transition(
            RuntimeState::Connecting,
            ReasonCode::Connecting,
            "resolving opaque credential reference",
        )
        .await?;
        let resolution = match self.credentials.resolve(&self.scope).await {
            Ok(resolution) => resolution,
            Err(error) => {
                self.transition(
                    RuntimeState::Halted,
                    ReasonCode::CredentialRejected,
                    "credential resolution failed",
                )
                .await?;
                return Err(RuntimeError::Broker(error));
            }
        };
        self.execution_authorized
            .store(resolution.execution_authorized, Ordering::SeqCst);
        self.update_health(|health| {
            health.execution_authorized = resolution.execution_authorized;
        })
        .await;
        if !resolution.execution_authorized {
            let audit = RuntimeAuditRecord {
                scope_key: self.scope.key(),
                runtime_epoch: epoch,
                event_type: "EXECUTION_AUTHORIZATION_REJECTED".into(),
                reason_code: ReasonCode::ExecutionUnauthorized,
                correlation_id: uuid::Uuid::new_v4().to_string(),
                redacted_detail: "credential resolved for reads; execution authorization disabled"
                    .into(),
                observed_at_unix_ms: now_unix_ms()?,
            };
            store_call(self.store.clone(), move |store| store.append_audit(&audit)).await?;
        }
        self.transition(
            RuntimeState::Reconciling,
            ReasonCode::ReconciliationStarted,
            "initial authoritative broker reconciliation",
        )
        .await?;

        let first = self.reconcile_locked().await?;
        if first.resulting_state == RuntimeState::Halted {
            self.apply_reconciliation_report(&first).await?;
            return Ok(first);
        }

        let (sender, receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        if let Err(error) = self.connect_streams(sender.clone()).await {
            self.transition(
                RuntimeState::Halted,
                ReasonCode::StreamDisconnected,
                "required execution stream connection failed after bounded retries",
            )
            .await?;
            return Err(error);
        }
        *self.stream_sender.lock().await = Some(sender);
        self.connected.store(true, Ordering::SeqCst);
        self.update_health(|health| health.connected = true).await;
        self.start_stream_worker(receiver).await;

        // Second snapshot closes event race across first snapshot and stream subscription.
        let final_report = self.reconcile_locked().await?;
        self.apply_reconciliation_report(&final_report).await?;
        Ok(final_report)
    }

    pub async fn dispatch(
        &self,
        command: RuntimeExecutionCommand,
        logical_request_id: impl Into<String>,
        correlation_id: impl Into<String>,
    ) -> Result<DispatchReceipt, RuntimeError> {
        let purpose = match self.scope.environment {
            crate::model::RuntimeEnvironment::Sandbox => RuntimeExecutionPurpose::SandboxMutation,
            crate::model::RuntimeEnvironment::Production => {
                RuntimeExecutionPurpose::ProductionAutomated
            }
        };
        self.dispatch_for_purpose(command, logical_request_id, correlation_id, purpose)
            .await
    }

    pub async fn dispatch_manual(
        &self,
        command: RuntimeExecutionCommand,
        logical_request_id: impl Into<String>,
        correlation_id: impl Into<String>,
    ) -> Result<DispatchReceipt, RuntimeError> {
        let purpose = match self.scope.environment {
            crate::model::RuntimeEnvironment::Sandbox => RuntimeExecutionPurpose::SandboxMutation,
            crate::model::RuntimeEnvironment::Production => {
                RuntimeExecutionPurpose::ProductionManual
            }
        };
        self.dispatch_for_purpose(command, logical_request_id, correlation_id, purpose)
            .await
    }

    async fn dispatch_for_purpose(
        &self,
        command: RuntimeExecutionCommand,
        logical_request_id: impl Into<String>,
        correlation_id: impl Into<String>,
        purpose: RuntimeExecutionPurpose,
    ) -> Result<DispatchReceipt, RuntimeError> {
        self.ensure_execution_allowed(purpose).await?;
        let logical_request_id = logical_request_id.into();
        if command.account_id() != self.scope.broker_account_id {
            return Err(RuntimeError::Safety(
                "execution command account does not match runtime scope".into(),
            ));
        }
        if command
            .request_identity()
            .is_some_and(|identity| identity != logical_request_id)
        {
            return Err(RuntimeError::Safety(
                "execution command request identity does not match runtime logical identity".into(),
            ));
        }
        let permit = self
            .command_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::CommandQueueFull)?;
        self.metrics.set_gauge(
            MetricName::RuntimeCommandQueueDepth,
            &[],
            (EXECUTION_QUEUE_CAPACITY - self.command_slots.available_permits()) as f64,
        );
        let result = self
            .dispatch_with_permit(permit, command, logical_request_id, correlation_id.into())
            .await;
        self.metrics.set_gauge(
            MetricName::RuntimeCommandQueueDepth,
            &[],
            (EXECUTION_QUEUE_CAPACITY - self.command_slots.available_permits()) as f64,
        );
        result
    }

    async fn dispatch_with_permit(
        &self,
        _permit: OwnedSemaphorePermit,
        command: RuntimeExecutionCommand,
        logical_request_id: String,
        correlation_id: String,
    ) -> Result<DispatchReceipt, RuntimeError> {
        let epoch = self.current_epoch()?;
        let now = now_unix_ms()?;
        let prepared = MutationRecord::prepared(
            &self.scope,
            logical_request_id.clone(),
            command.kind(),
            command.evidence(),
            correlation_id,
            epoch,
            now,
        )?;
        let inserted = store_call(self.store.clone(), {
            let prepared = prepared.clone();
            move |store| store.insert_mutation(&prepared)
        })
        .await;
        self.require_store(inserted, "failed to persist mutation intent")
            .await?;
        let fenced = store_call(self.store.clone(), {
            let scope_key = self.scope.key();
            let logical_request_id = logical_request_id.clone();
            move |store| store.claim_dispatch_unknown(&scope_key, &logical_request_id, epoch, now)
        })
        .await;
        let fenced = self
            .require_store(fenced, "failed to persist pre-dispatch UNKNOWN fence")
            .await?;
        tracing::info!(
            event = "mutation_dispatch_fenced",
            runtime_epoch = epoch,
            provider = ?self.scope.provider,
            environment = ?self.scope.environment,
            account_scope = %self.scope.redacted_account_id(),
            logical_request_id = %logical_request_id,
            mutation_kind = ?command.kind(),
        );

        let result = self
            .execution
            .dispatch_once(&self.scope, &command, &fenced)
            .await;
        match result {
            Ok(ExecutionResult::Acknowledged {
                broker_evidence_ref,
                links,
            }) => {
                let linked = store_call(self.store.clone(), {
                    let scope_key = self.scope.key();
                    let links = links.clone();
                    move |store| {
                        store.upsert_identity_links(&scope_key, &links, epoch, now_unix_ms_store()?)
                    }
                })
                .await;
                self.require_store(
                    linked,
                    "broker acknowledged mutation but identity-link persistence failed",
                )
                .await?;
                let record = self
                    .finish_mutation(
                        &logical_request_id,
                        JournalState::Acknowledged,
                        Some(&broker_evidence_ref),
                        None,
                    )
                    .await?;
                Ok(receipt(record))
            }
            Ok(ExecutionResult::Rejected {
                broker_evidence_ref,
            }) => {
                let record = self
                    .finish_mutation(
                        &logical_request_id,
                        JournalState::Rejected,
                        Some(&broker_evidence_ref),
                        None,
                    )
                    .await?;
                Ok(receipt(record))
            }
            Ok(ExecutionResult::UnknownAfterDispatch {
                broker_evidence_ref,
            }) => {
                if broker_evidence_ref.is_some() {
                    self.finish_mutation(
                        &logical_request_id,
                        JournalState::UnknownAfterDispatch,
                        broker_evidence_ref.as_deref(),
                        None,
                    )
                    .await?;
                }
                self.transition(
                    RuntimeState::Halted,
                    ReasonCode::UnknownMutation,
                    "capital mutation remains UNKNOWN_AFTER_DISPATCH",
                )
                .await?;
                Err(RuntimeError::UnknownAfterDispatch(logical_request_id))
            }
            Err(error) => {
                self.transition(
                    RuntimeState::Halted,
                    ReasonCode::UnknownMutation,
                    "mutation transport failed after durable dispatch fence",
                )
                .await?;
                Err(RuntimeError::UnknownTransport {
                    logical_request_id,
                    source: error,
                })
            }
        }
    }

    async fn finish_mutation(
        &self,
        logical_request_id: &str,
        target: JournalState,
        broker_evidence_ref: Option<&str>,
        disposition: Option<&crate::model::ReconciliationDisposition>,
    ) -> Result<MutationRecord, RuntimeError> {
        let epoch = self.current_epoch()?;
        let scope_key = self.scope.key();
        let logical_request_id = logical_request_id.to_owned();
        let broker_evidence_ref = broker_evidence_ref.map(str::to_owned);
        let disposition = disposition.cloned();
        let result = store_call(self.store.clone(), move |store| {
            store.transition_mutation(
                &scope_key,
                &logical_request_id,
                &[JournalState::UnknownAfterDispatch],
                target,
                broker_evidence_ref.as_deref(),
                disposition.as_ref(),
                epoch,
                now_unix_ms_store()?,
            )
        })
        .await;
        self.require_store(result, "mutation outcome persistence failed")
            .await
    }

    pub async fn force_reconciliation(
        self: &Arc<Self>,
        reason_code: ReasonCode,
        detail: &str,
    ) -> Result<ReconciliationReport, RuntimeError> {
        self.transition(RuntimeState::Reconciling, reason_code, detail)
            .await?;
        let report = self.reconcile_locked().await?;
        self.apply_reconciliation_report(&report).await?;
        Ok(report)
    }

    pub async fn report_stream_overflow(
        self: &Arc<Self>,
        stream: StreamKind,
    ) -> Result<ReconciliationReport, RuntimeError> {
        self.metrics.increment(
            MetricName::StreamDroppedTotal,
            &[
                MetricLabel::Stream(stream),
                MetricLabel::Reason(ReasonCode::StreamQueueOverflow),
            ],
            1,
        );
        self.force_reconciliation(
            ReasonCode::StreamQueueOverflow,
            "safety-critical stream queue overflowed; broker snapshot required",
        )
        .await
    }

    pub async fn shutdown(self: &Arc<Self>) -> Result<(), RuntimeError> {
        self.transition(
            RuntimeState::Stopping,
            ReasonCode::ShutdownRequested,
            "new command admission closed",
        )
        .await?;
        let drain = self
            .command_slots
            .clone()
            .acquire_many_owned(u32::try_from(EXECUTION_QUEUE_CAPACITY).map_err(|_| {
                RuntimeError::Safety("execution queue capacity exceeds u32".into())
            })?);
        let _drained = tokio::time::timeout(self.config.shutdown_timeout, drain).await;
        let _ = self.streams.disconnect().await;
        self.stream_sender.lock().await.take();
        if let Some(task) = self.stream_task.lock().await.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.coalesced_reconciliation_task.lock().await.take() {
            task.abort();
            let _ = task.await;
        }
        self.connected.store(false, Ordering::SeqCst);
        self.transition(
            RuntimeState::Stopped,
            ReasonCode::ShutdownComplete,
            "stream tasks closed and final audit persisted",
        )
        .await?;
        let epoch = self.current_epoch()?;
        store_call(self.store.clone(), {
            let scope_key = self.scope.key();
            move |store| store.release_ownership(&scope_key, epoch, now_unix_ms_store()?)
        })
        .await?;
        self.update_health(|health| {
            health.connected = false;
            health.new_exposure_allowed = false;
        })
        .await;
        Ok(())
    }

    async fn ensure_execution_allowed(
        &self,
        purpose: RuntimeExecutionPurpose,
    ) -> Result<(), RuntimeError> {
        let state = self.state.lock().await.state();
        if state != RuntimeState::Ready
            || !self.connected.load(Ordering::SeqCst)
            || self.reconciliation_dirty.load(Ordering::SeqCst)
            || self.reconciliation_running.load(Ordering::SeqCst)
        {
            return Err(RuntimeError::ExecutionGateClosed);
        }
        if self
            .credentials
            .authorize_execution(&self.scope, purpose)
            .await
            .is_err()
        {
            self.execution_authorized.store(false, Ordering::SeqCst);
            self.update_health(|health| {
                health.execution_authorized = false;
                health.new_exposure_allowed = false;
            })
            .await;
            return Err(RuntimeError::ExecutionGateClosed);
        }
        if purpose != RuntimeExecutionPurpose::ProductionManual {
            self.execution_authorized.store(true, Ordering::SeqCst);
            self.update_health(|health| health.execution_authorized = true)
                .await;
        }
        let verified = store_call(self.store.clone(), {
            let scope_key = self.scope.key();
            let epoch = self.current_epoch()?;
            move |store| store.verify_epoch(&scope_key, epoch)
        })
        .await;
        self.require_store(verified, "runtime ownership/epoch verification failed")
            .await?;
        Ok(())
    }

    async fn require_store<T>(
        &self,
        result: Result<T, StoreError>,
        detail: &str,
    ) -> Result<T, RuntimeError> {
        match result {
            Ok(value) => Ok(value),
            Err(source @ StoreError::DuplicateMutation) => Err(RuntimeError::Store(source)),
            Err(source) => {
                if self
                    .transition(RuntimeState::Halted, ReasonCode::PersistenceFailure, detail)
                    .await
                    .is_err()
                {
                    self.update_health(|health| {
                        health.state = RuntimeState::Halted;
                        health.reason_code = ReasonCode::PersistenceFailure;
                        health.reason = detail.to_owned();
                        health.persistence_healthy = false;
                        health.new_exposure_allowed = false;
                    })
                    .await;
                    self.metrics.set_gauge(MetricName::RuntimeReady, &[], 0.0);
                }
                self.metrics.increment(
                    MetricName::PersistenceErrorsTotal,
                    &[MetricLabel::Operation(
                        crate::ports::StoreOperation::Mutation,
                    )],
                    1,
                );
                Err(RuntimeError::Store(source))
            }
        }
    }

    async fn reconcile_locked(&self) -> Result<ReconciliationReport, RuntimeError> {
        let _guard = self.reconciliation_guard.lock().await;
        let epoch = self.current_epoch()?;
        tracing::info!(
            event = "reconciliation_started",
            runtime_epoch = epoch,
            provider = ?self.scope.provider,
            environment = ?self.scope.environment,
            account_scope = %self.scope.redacted_account_id(),
        );
        match self.reconciler.reconcile(&self.scope, epoch).await {
            Ok(report) => {
                tracing::info!(
                    event = "reconciliation_completed",
                    runtime_epoch = epoch,
                    reconciliation_id = %report.reconciliation_id,
                    resulting_state = ?report.resulting_state,
                    unresolved_unknown_count = report.unresolved_logical_request_ids.len(),
                );
                Ok(report)
            }
            Err(error) => {
                tracing::error!(
                    event = "reconciliation_failed",
                    runtime_epoch = epoch,
                    reason_code = ?reconciliation_error_reason(&error),
                    error = %error,
                );
                let reason = reconciliation_error_reason(&error);
                if self
                    .transition(
                        RuntimeState::Halted,
                        reason,
                        "authoritative broker reconciliation failed",
                    )
                    .await
                    .is_err()
                {
                    self.update_health(|health| {
                        health.state = RuntimeState::Halted;
                        health.reason_code = reason;
                        health.reason = "authoritative broker reconciliation failed".into();
                        health.persistence_healthy = false;
                        health.new_exposure_allowed = false;
                    })
                    .await;
                    self.metrics.set_gauge(MetricName::RuntimeReady, &[], 0.0);
                }
                Err(RuntimeError::Reconciliation(error))
            }
        }
    }

    async fn apply_reconciliation_report(
        &self,
        report: &ReconciliationReport,
    ) -> Result<(), RuntimeError> {
        self.update_health(|health| {
            health.last_successful_reconciliation_at_unix_ms = Some(report.completed_at_unix_ms);
            health.reconciliation_age_ms = Some(0);
            health.unresolved_unknown_count =
                u64::try_from(report.unresolved_logical_request_ids.len()).unwrap_or(u64::MAX);
            health.open_order_count = u64::try_from(report.active_order_count).unwrap_or(u64::MAX);
            health.active_stop_count = u64::try_from(report.active_stop_count).unwrap_or(u64::MAX);
        })
        .await;
        let detail = if report.discrepancies.is_empty() {
            "authoritative snapshot, stream handoff and checkpoint complete".to_owned()
        } else {
            report.discrepancies.join("; ")
        };
        self.transition(report.resulting_state, report.reason_code, &detail)
            .await
    }

    async fn transition(
        &self,
        target: RuntimeState,
        reason_code: ReasonCode,
        detail: &str,
    ) -> Result<(), RuntimeError> {
        let epoch = self.epoch.load(Ordering::SeqCst);
        let mut state = self.state.lock().await;
        let from = state.transition(target)?;
        if epoch > 0 {
            let transition = StateTransition {
                scope_key: self.scope.key(),
                from,
                to: target,
                reason_code,
                detail: detail.to_owned(),
                correlation_id: uuid::Uuid::new_v4().to_string(),
                observed_at_unix_ms: now_unix_ms()?,
                runtime_epoch: epoch,
            };
            store_call(self.store.clone(), move |store| {
                store.record_transition(&transition)
            })
            .await?;
        }
        tracing::info!(
            event = "runtime_state_changed",
            runtime_epoch = epoch,
            provider = ?self.scope.provider,
            environment = ?self.scope.environment,
            account_scope = %self.scope.redacted_account_id(),
            from = ?from,
            to = ?target,
            reason_code = ?reason_code,
        );
        let execution_authorized = self.execution_authorized.load(Ordering::SeqCst);
        let connected = self.connected.load(Ordering::SeqCst);
        self.update_health(|health| {
            health.state = target;
            health.reason_code = reason_code;
            health.reason = detail.to_owned();
            health.connected = connected;
            health.execution_authorized = execution_authorized;
            health.new_exposure_allowed =
                target.new_exposure_allowed() && execution_authorized && connected;
            health.persistence_healthy = true;
        })
        .await;
        self.metrics
            .set_gauge(MetricName::RuntimeState, &[], state_number(target));
        self.metrics.set_gauge(
            MetricName::RuntimeReady,
            &[],
            if target == RuntimeState::Ready {
                1.0
            } else {
                0.0
            },
        );
        Ok(())
    }

    async fn start_stream_worker(self: &Arc<Self>, mut receiver: mpsc::Receiver<StreamSignal>) {
        let coordinator = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            while let Some(signal) = receiver.recv().await {
                let Some(coordinator) = coordinator.upgrade() else {
                    return;
                };
                let queue_depth = receiver.len();
                if coordinator
                    .handle_stream_signal(signal, queue_depth)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        *self.stream_task.lock().await = Some(task);
    }

    async fn handle_stream_signal(
        self: &Arc<Self>,
        signal: StreamSignal,
        queue_depth: usize,
    ) -> Result<(), RuntimeError> {
        match signal {
            StreamSignal::Connected(stream) => {
                self.set_stream_state(stream, StreamState::Active, 0).await;
            }
            StreamSignal::Event(event) => {
                let event_class = event.event_class;
                let stream = event_class_stream(event_class);
                self.set_stream_state(stream, StreamState::Active, queue_depth)
                    .await;
                self.update_health(|health| {
                    if let Some(stream_health) = health
                        .stream_states
                        .iter_mut()
                        .find(|candidate| candidate.stream == stream)
                    {
                        stream_health.last_event_at_unix_ms = now_unix_ms().ok();
                    }
                })
                .await;
                if event.runtime_epoch != self.current_epoch()? {
                    self.metrics.increment(
                        MetricName::StreamDroppedTotal,
                        &[
                            MetricLabel::Stream(event_class_stream(event.event_class)),
                            MetricLabel::Reason(ReasonCode::StaleEpoch),
                        ],
                        1,
                    );
                    return Ok(());
                }
                let event_for_store = event.clone();
                let inserted = store_call(self.store.clone(), {
                    let scope_key = self.scope.key();
                    move |store| {
                        store.record_broker_event(
                            &scope_key,
                            &event_for_store,
                            now_unix_ms_store()?,
                        )
                    }
                })
                .await?;
                if !inserted {
                    self.metrics.increment(
                        MetricName::EventDeduplicatedTotal,
                        &[MetricLabel::EventClass(event_class)],
                        1,
                    );
                } else if matches!(
                    event_class,
                    crate::model::BrokerEventClass::Order | crate::model::BrokerEventClass::Stop
                ) && !self.stream_event_has_local_provenance(&event).await?
                {
                    let reason = if event_class == crate::model::BrokerEventClass::Order {
                        ReasonCode::BrokerOrderConflict
                    } else {
                        ReasonCode::BrokerStopConflict
                    };
                    let report = self
                        .force_reconciliation(
                            reason,
                            "unfamiliar capital-affecting stream identity requires broker snapshot",
                        )
                        .await?;
                    if report.resulting_state == RuntimeState::Ready {
                        self.transition(
                            RuntimeState::Halted,
                            reason,
                            "unfamiliar stream identity not yet confirmed by unary snapshot",
                        )
                        .await?;
                    }
                } else if matches!(
                    event_class,
                    crate::model::BrokerEventClass::Fill
                        | crate::model::BrokerEventClass::Operation
                        | crate::model::BrokerEventClass::Position
                        | crate::model::BrokerEventClass::Portfolio
                ) {
                    self.request_coalesced_reconciliation(
                        ReasonCode::ReconciliationStarted,
                        "capital-state stream event requires authoritative unary refresh",
                    )
                    .await?;
                }
            }
            StreamSignal::Gap { stream, reason }
            | StreamSignal::Disconnected { stream, reason } => {
                self.set_stream_state(stream, StreamState::Stale, 0).await;
                self.metrics.increment(
                    MetricName::StreamReconnectTotal,
                    &[MetricLabel::Stream(stream)],
                    1,
                );
                if required_stream(stream) {
                    self.connected.store(false, Ordering::SeqCst);
                    self.update_health(|health| {
                        health.connected = false;
                        health.new_exposure_allowed = false;
                    })
                    .await;
                    self.recover_stream_gap(&reason).await?;
                }
            }
        }
        Ok(())
    }

    async fn request_coalesced_reconciliation(
        self: &Arc<Self>,
        reason_code: ReasonCode,
        detail: &str,
    ) -> Result<(), RuntimeError> {
        self.reconciliation_dirty.store(true, Ordering::SeqCst);
        if self
            .reconciliation_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }
        self.transition(RuntimeState::Reconciling, reason_code, detail)
            .await?;
        let coordinator = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            let Some(coordinator) = coordinator.upgrade() else {
                return;
            };
            loop {
                coordinator
                    .reconciliation_dirty
                    .store(false, Ordering::SeqCst);
                let report = match coordinator.reconcile_locked().await {
                    Ok(report) => report,
                    Err(_) => {
                        coordinator
                            .reconciliation_running
                            .store(false, Ordering::SeqCst);
                        return;
                    }
                };
                if coordinator.reconciliation_dirty.load(Ordering::SeqCst) {
                    continue;
                }
                if coordinator
                    .apply_reconciliation_report(&report)
                    .await
                    .is_err()
                {
                    coordinator
                        .reconciliation_running
                        .store(false, Ordering::SeqCst);
                    return;
                }
                coordinator
                    .reconciliation_running
                    .store(false, Ordering::SeqCst);
                if !coordinator.reconciliation_dirty.load(Ordering::SeqCst)
                    || coordinator
                        .reconciliation_running
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                {
                    return;
                }
                if coordinator
                    .transition(
                        RuntimeState::Reconciling,
                        ReasonCode::ReconciliationStarted,
                        "capital state changed during authoritative refresh",
                    )
                    .await
                    .is_err()
                {
                    coordinator
                        .reconciliation_running
                        .store(false, Ordering::SeqCst);
                    return;
                }
            }
        });
        *self.coalesced_reconciliation_task.lock().await = Some(task);
        Ok(())
    }

    async fn stream_event_has_local_provenance(
        &self,
        event: &crate::model::BrokerEvent,
    ) -> Result<bool, RuntimeError> {
        let scope_key = self.scope.key();
        let links = store_call(self.store.clone(), {
            let scope_key = scope_key.clone();
            move |store| store.all_identity_links(&scope_key)
        })
        .await?;
        if links.iter().any(|links| {
            event.broker_order_id.as_deref().is_some_and(|event_id| {
                links.broker_order_id.as_deref() == Some(event_id)
                    || links.replacement_broker_order_id.as_deref() == Some(event_id)
            }) || event
                .broker_stop_order_id
                .as_deref()
                .is_some_and(|event_id| links.broker_stop_order_id.as_deref() == Some(event_id))
        }) {
            return Ok(true);
        }
        let unresolved = store_call(self.store.clone(), move |store| {
            store.unresolved_mutations(&scope_key)
        })
        .await?;
        Ok(event
            .logical_request_id
            .as_deref()
            .is_some_and(|request_id| {
                unresolved
                    .iter()
                    .any(|mutation| mutation.logical_request_id == request_id)
            }))
    }

    async fn recover_stream_gap(self: &Arc<Self>, reason: &str) -> Result<(), RuntimeError> {
        self.transition(RuntimeState::Reconciling, ReasonCode::StreamGap, reason)
            .await?;
        let first = self.reconcile_locked().await?;
        if first.resulting_state == RuntimeState::Halted {
            self.apply_reconciliation_report(&first).await?;
            return Ok(());
        }
        let sender = self
            .stream_sender
            .lock()
            .await
            .clone()
            .ok_or_else(|| RuntimeError::Safety("stream sender unavailable".into()))?;
        if let Err(error) = self.connect_streams(sender).await {
            self.transition(
                RuntimeState::Halted,
                ReasonCode::StreamDisconnected,
                "execution stream reconnect failed after bounded retries",
            )
            .await?;
            return Err(error);
        }
        let final_report = self.reconcile_locked().await?;
        self.apply_reconciliation_report(&final_report).await
    }

    async fn connect_streams(
        &self,
        sender: mpsc::Sender<StreamSignal>,
    ) -> Result<(), RuntimeError> {
        let epoch = self.current_epoch()?;
        let mut delay = Duration::from_millis(250);
        for attempt in 1..=3_u8 {
            match self
                .streams
                .connect(&self.scope, epoch, sender.clone())
                .await
            {
                Ok(active_streams) if required_streams().is_subset(&active_streams) => {
                    for stream in active_streams {
                        self.set_stream_state(stream, StreamState::Active, 0).await;
                    }
                    self.connected.store(true, Ordering::SeqCst);
                    return Ok(());
                }
                Ok(active_streams) => {
                    let missing = required_streams()
                        .difference(&active_streams)
                        .map(|stream| format!("{stream:?}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    return Err(RuntimeError::Safety(format!(
                        "required stream ACK set incomplete: {missing}"
                    )));
                }
                Err(error) if attempt < 3 && error.safe_read_retryable() => {
                    tokio::time::sleep(jittered_delay(delay, Duration::from_secs(2))).await;
                    delay = delay.saturating_mul(2).min(Duration::from_secs(2));
                }
                Err(error) => return Err(RuntimeError::Broker(error)),
            }
        }
        Err(RuntimeError::Safety(
            "bounded stream reconnect attempts exhausted".into(),
        ))
    }

    async fn set_stream_state(&self, stream: StreamKind, state: StreamState, queue_depth: usize) {
        self.update_health(|health| {
            if let Some(stream_health) = health
                .stream_states
                .iter_mut()
                .find(|candidate| candidate.stream == stream)
            {
                stream_health.state = state;
                stream_health.queue_depth = queue_depth;
            }
        })
        .await;
        self.metrics.set_gauge(
            MetricName::StreamConnections,
            &[MetricLabel::Stream(stream)],
            if state == StreamState::Active {
                1.0
            } else {
                0.0
            },
        );
        self.metrics.set_gauge(
            MetricName::StreamQueueDepth,
            &[MetricLabel::Stream(stream)],
            queue_depth as f64,
        );
    }

    async fn update_health(&self, update: impl FnOnce(&mut RuntimeHealth)) {
        let mut health = self.health.write().await;
        update(&mut health);
    }

    fn current_epoch(&self) -> Result<u64, RuntimeError> {
        let epoch = self.epoch.load(Ordering::SeqCst);
        if epoch == 0 {
            Err(RuntimeError::ExecutionGateClosed)
        } else {
            Ok(epoch)
        }
    }
}

#[async_trait]
impl<R, E, X, C, S, M> HealthReadPort for RuntimeCoordinator<R, E, X, C, S, M>
where
    R: crate::ports::BrokerReadPort + 'static,
    E: ExecutionPort + 'static,
    X: ExecutionStreamPort + 'static,
    C: CredentialResolverPort + 'static,
    S: RuntimeStore,
    M: MetricsPort + 'static,
{
    async fn health(&self) -> RuntimeHealth {
        let mut health = self.health.read().await.clone();
        health.reconciliation_age_ms =
            health
                .last_successful_reconciliation_at_unix_ms
                .and_then(|last| {
                    now_unix_ms()
                        .ok()
                        .and_then(|now| u64::try_from(now - last).ok())
                });
        health
    }
}

fn all_stream_health() -> Vec<StreamHealth> {
    [
        StreamKind::OrderState,
        StreamKind::Trades,
        StreamKind::Positions,
        StreamKind::Portfolio,
        StreamKind::Operations,
    ]
    .into_iter()
    .map(|stream| StreamHealth {
        stream,
        required_for_ready: required_stream(stream),
        state: StreamState::Disconnected,
        queue_depth: 0,
        last_event_at_unix_ms: None,
    })
    .collect()
}

fn required_streams() -> BTreeSet<StreamKind> {
    [
        StreamKind::OrderState,
        StreamKind::Positions,
        StreamKind::Portfolio,
        StreamKind::Operations,
    ]
    .into_iter()
    .collect()
}

const fn required_stream(stream: StreamKind) -> bool {
    matches!(
        stream,
        StreamKind::OrderState
            | StreamKind::Positions
            | StreamKind::Portfolio
            | StreamKind::Operations
    )
}

fn receipt(record: MutationRecord) -> DispatchReceipt {
    DispatchReceipt {
        logical_request_id: record.logical_request_id,
        state: record.state,
        broker_evidence_ref: record.broker_evidence_ref,
    }
}

fn reconciliation_error_reason(error: &ReconciliationError) -> ReasonCode {
    match error {
        ReconciliationError::Broker(error) => match error.class {
            crate::ports::BrokerResultClass::Credential => ReasonCode::CredentialRejected,
            _ => ReasonCode::RequiredReadUnavailable,
        },
        ReconciliationError::Store(_) => ReasonCode::PersistenceFailure,
        ReconciliationError::Safety(_) => ReasonCode::ReconciliationIncomplete,
    }
}

fn event_class_stream(class: crate::model::BrokerEventClass) -> StreamKind {
    match class {
        crate::model::BrokerEventClass::Order | crate::model::BrokerEventClass::Stop => {
            StreamKind::OrderState
        }
        crate::model::BrokerEventClass::Fill => StreamKind::Trades,
        crate::model::BrokerEventClass::Operation => StreamKind::Operations,
        crate::model::BrokerEventClass::Position => StreamKind::Positions,
        crate::model::BrokerEventClass::Portfolio => StreamKind::Portfolio,
    }
}

const fn state_number(state: RuntimeState) -> f64 {
    match state {
        RuntimeState::Starting => 0.0,
        RuntimeState::Connecting => 1.0,
        RuntimeState::Reconciling => 2.0,
        RuntimeState::Ready => 3.0,
        RuntimeState::Degraded => 4.0,
        RuntimeState::Halted => 5.0,
        RuntimeState::Stopping => 6.0,
        RuntimeState::Stopped => 7.0,
    }
}

fn now_unix_ms() -> Result<i64, RuntimeError> {
    now_unix_ms_store().map_err(RuntimeError::Store)
}

fn now_unix_ms_store() -> Result<i64, StoreError> {
    let nanos = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000)
        .map_err(|_| StoreError::Corrupt("system clock is outside i64 range".into()))
}

fn jittered_delay(base: Duration, maximum: Duration) -> Duration {
    let spread = base.as_nanos() / 4;
    let byte = u128::from(uuid::Uuid::new_v4().as_bytes()[0]);
    let jitter = spread.saturating_mul(byte) / 255;
    Duration::from_nanos(u64::try_from(base.as_nanos().saturating_add(jitter)).unwrap_or(u64::MAX))
        .min(maximum)
}

async fn store_call<S, T, F>(store: S, operation: F) -> Result<T, StoreError>
where
    S: RuntimeStore,
    T: Send + 'static,
    F: FnOnce(S) -> Result<T, StoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || operation(store))
        .await
        .map_err(|error| StoreError::BlockingTask(error.to_string()))?
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Broker(#[from] BrokerPortError),
    #[error(transparent)]
    Reconciliation(#[from] ReconciliationError),
    #[error(transparent)]
    Transition(#[from] crate::policy::TransitionError),
    #[error(transparent)]
    Model(#[from] crate::model::ModelError),
    #[error("runtime execution gate is closed")]
    ExecutionGateClosed,
    #[error("runtime command queue is full")]
    CommandQueueFull,
    #[error("mutation {0} remains UNKNOWN_AFTER_DISPATCH")]
    UnknownAfterDispatch(String),
    #[error("mutation {logical_request_id} transport outcome is unknown: {source}")]
    UnknownTransport {
        logical_request_id: String,
        source: BrokerPortError,
    },
    #[error("runtime safety failure: {0}")]
    Safety(String),
}
