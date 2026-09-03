use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock, Semaphore};
use vox_connections::{ConnectionId, ExecutionPurpose};
use vox_domain::{OrderSide, RegularOrderType, RuntimeExecutionCommand};
use vox_risk::{
    BrokerLotLimits, BrokerMarginFacts, BuyLotLimit, ProtectionPlanState, ReservationCapacity,
    ReservationState, RiskActionKind, RiskEngine, RiskPolicySet, RiskReasonCode, RiskRequest,
    RiskReservation, RiskReservationReconciler, RiskSnapshot, RiskSource, RiskState, RiskStore,
    RiskValidityContext, SellLotLimit, SqliteRiskStore,
};
use vox_runtime::{
    ReconciliationReport, RiskAdmission, RiskAdmissionError, RiskAdmissionPort,
    RiskDispatchOutcome, RuntimeExecutionPurpose, RuntimeScope, RuntimeState, RuntimeStore,
    SqliteRuntimeStore,
};

use crate::composition::ProductionClientFactory;

#[derive(Clone)]
pub struct ProductionRiskSnapshot {
    pub report: ReconciliationReport,
}

pub struct ProductionRiskAdapter {
    canonical_account_id: String,
    broker_connection_id: String,
    runtime_store: SqliteRuntimeStore,
    risk_store: SqliteRiskStore,
    factory: Arc<ProductionClientFactory>,
    snapshot: RwLock<Option<ProductionRiskSnapshot>>,
    instrument_cache:
        Mutex<HashMap<String, (i64, vox_tinvest::risk_read::CanonicalRiskInstrument)>>,
    broker_query_slots: Semaphore,
}

impl ProductionRiskAdapter {
    #[must_use]
    pub fn new(
        canonical_account_id: String,
        broker_connection_id: String,
        runtime_store: SqliteRuntimeStore,
        risk_store: SqliteRiskStore,
        factory: Arc<ProductionClientFactory>,
    ) -> Self {
        Self {
            canonical_account_id,
            broker_connection_id,
            runtime_store,
            risk_store,
            factory,
            snapshot: RwLock::new(None),
            instrument_cache: Mutex::new(HashMap::new()),
            broker_query_slots: Semaphore::new(4),
        }
    }

    pub fn policy(&self) -> Result<RiskPolicySet, RiskAdmissionError> {
        if let Some(policy) = self
            .risk_store
            .policy(&self.canonical_account_id)
            .map_err(store_unavailable)?
        {
            return Ok(policy);
        }
        let policy = initial_policy();
        self.risk_store
            .put_policy(
                &self.canonical_account_id,
                None,
                &policy,
                "initialize fail-closed account policy",
                now_unix_ms()?,
            )
            .map_err(store_unavailable)
    }

    /// Builds broker-authoritative protection evidence for the requesting instrument,
    /// correlated to the durable #10 protection plan when one exists. Coverage is
    /// derived from broker stop facts (`report.active_stops`) and current position lots,
    /// never from local intent.
    async fn protection_status(
        &self,
        report: &ReconciliationReport,
        instrument_id: &str,
    ) -> Result<vox_risk::RiskProtectionStatus, RiskAdmissionError> {
        let position_lots = report
            .positions
            .iter()
            .find(|position| position.instrument_uid == instrument_id)
            .map(|position| position.quantity_units)
            .unwrap_or(0);

        let active_stop_lots = report
            .active_stops
            .iter()
            .filter(|stop| stop.instrument_uid == instrument_id && stop.status.active())
            .map(|stop| stop.quantity_lots.unwrap_or(0))
            .sum::<i64>();

        let position_entered_at_unix_ms = report
            .positions
            .iter()
            .find(|position| position.instrument_uid == instrument_id)
            .and_then(|position| position.broker_observed_at_unix_ms);

        let plans = self
            .risk_store
            .protection_plans_for_instrument(&self.canonical_account_id, instrument_id)
            .map_err(store_unavailable)?;
        let (plan_id, plan_state) = plans
            .iter()
            .filter(|plan| !plan.state.terminal())
            .max_by_key(|plan| plan.created_at_unix_ms)
            .map(|plan| (Some(plan.plan_id.clone()), Some(plan.state)))
            .unwrap_or((None, None));

        Ok(vox_risk::RiskProtectionStatus {
            active_stop_lots,
            position_lots,
            position_entered_at_unix_ms,
            plan_id,
            plan_state,
        })
    }

    pub fn replace_state(
        &self,
        expected_revision: u64,
        state: RiskState,
        reason: &str,
    ) -> Result<RiskPolicySet, RiskAdmissionError> {
        let mut policy = self.policy()?;
        if policy.revision != expected_revision {
            return Err(RiskAdmissionError::Stale(
                "risk policy revision changed".into(),
            ));
        }
        policy.revision = policy
            .revision
            .checked_add(1)
            .ok_or_else(|| RiskAdmissionError::Unavailable("policy revision overflow".into()))?;
        policy.state = state;
        self.risk_store
            .put_policy(
                &self.canonical_account_id,
                Some(expected_revision),
                &policy,
                reason,
                now_unix_ms()?,
            )
            .map_err(|error| match error {
                vox_risk::RiskStoreError::PolicyRevisionConflict => {
                    RiskAdmissionError::Stale("risk policy revision changed".into())
                }
                other => store_unavailable(other),
            })
    }

    pub fn active_reservations(&self) -> Result<Vec<RiskReservation>, RiskAdmissionError> {
        self.risk_store
            .active_reservations(&self.canonical_account_id)
            .map_err(store_unavailable)
    }

    pub fn decision_for_request(
        &self,
        logical_request_id: &str,
    ) -> Result<Option<vox_risk::RiskDecision>, RiskAdmissionError> {
        self.risk_store
            .decision_for_request(&self.canonical_account_id, logical_request_id)
            .map_err(store_unavailable)
    }

    async fn instrument_constraints(
        &self,
        reads: &vox_tinvest::risk_read::TInvestRiskReadAdapter,
        instrument_id: &str,
        now_unix_ms: i64,
    ) -> Result<(vox_tinvest::risk_read::CanonicalRiskInstrument, u64), RiskAdmissionError> {
        let mut cache = self.instrument_cache.lock().await;
        if let Some((cached_at, value)) = cache.get(instrument_id)
            && now_unix_ms.saturating_sub(*cached_at) <= 1_000
        {
            return Ok((value.clone(), risk_revision(*cached_at)?));
        }
        let value = reads
            .instrument_constraints(instrument_id)
            .await
            .map_err(|error| RiskAdmissionError::Denied {
                code: "INSTRUMENT_UNAVAILABLE".into(),
                message: error.to_string(),
            })?;
        cache.insert(instrument_id.to_owned(), (now_unix_ms, value.clone()));
        Ok((value, risk_revision(now_unix_ms)?))
    }

    async fn build_request(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
        command: &RuntimeExecutionCommand,
        logical_request_id: &str,
        policy: &RiskPolicySet,
    ) -> Result<RiskRequest, RiskAdmissionError> {
        let cached = self.snapshot.read().await.clone().ok_or_else(|| {
            RiskAdmissionError::Unavailable(
                "risk has no broker-authoritative reconciliation snapshot".into(),
            )
        })?;
        if cached.report.runtime_epoch == 0 {
            return Err(RiskAdmissionError::Unavailable(
                "risk snapshot runtime epoch is invalid".into(),
            ));
        }
        let connection_id = ConnectionId::parse(scope.connection_ref.as_str().to_owned())
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        let execution_purpose = map_purpose(purpose);
        let execution = self
            .factory
            .execution_session(&connection_id, &scope.broker_account_id, execution_purpose)
            .map_err(|error| RiskAdmissionError::Denied {
                code: "EXECUTION_UNAUTHORIZED".into(),
                message: error.to_string(),
            })?;

        let description = command_description(command, &cached.report)?;
        let now = now_unix_ms()?;
        let directional = matches!(
            description.action,
            RiskActionKind::DirectionalOrder | RiskActionKind::ReplaceDirectionalOrder
        );
        let read_session = self
            .factory
            .read_session(&connection_id, &scope.broker_account_id)
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        let _broker_query_permit =
            if directional {
                Some(self.broker_query_slots.acquire().await.map_err(|_| {
                    RiskAdmissionError::Unavailable("risk query gate closed".into())
                })?)
            } else {
                None
            };

        let (
            instrument_tradable,
            lot_size,
            broker_lot_limits,
            current_position_lots,
            instrument_constraints_revision,
        ) = if directional {
            let (constraints, constraints_revision) = self
                .instrument_constraints(&read_session.risk_reads, &description.instrument_id, now)
                .await?;
            let side_allowed = match description.delta_lots.signum() {
                1 => constraints.buy_available,
                -1 => constraints.sell_available,
                _ => false,
            };
            let order_type_allowed = match description.order_type {
                Some(RegularOrderType::Limit) => constraints.limit_order_available,
                Some(RegularOrderType::Market) => constraints.market_order_available,
                Some(RegularOrderType::BestPrice) => constraints.best_price_order_available,
                None => true,
            };
            let current_units = cached
                .report
                .positions
                .iter()
                .filter(|position| position.instrument_uid == description.instrument_id)
                .try_fold(0_i64, |total, position| {
                    total.checked_add(position.quantity_units).ok_or_else(|| {
                        RiskAdmissionError::Unavailable("position quantity overflow".into())
                    })
                })?;
            if current_units % constraints.lot_size != 0 {
                return Err(RiskAdmissionError::Denied {
                    code: "POSITION_LOT_MISMATCH".into(),
                    message: "broker position is not exactly divisible by instrument lot".into(),
                });
            }
            let limits = read_session
                .risk_reads
                .max_lots(
                    &scope.broker_account_id,
                    &description.instrument_id,
                    description.price,
                )
                .await
                .map_err(|error| RiskAdmissionError::Denied {
                    code: "PROVIDER_LIMIT_UNAVAILABLE".into(),
                    message: error.to_string(),
                })?;
            (
                constraints.api_trade_available && side_allowed && order_type_allowed,
                constraints.lot_size,
                Some(map_lot_limits(limits)),
                current_units / constraints.lot_size,
                constraints_revision,
            )
        } else {
            (true, 1, None, 0, 0)
        };

        let open_order_delta_lots = cached
            .report
            .active_orders
            .iter()
            .filter(|order| order.instrument_uid == description.instrument_id)
            .filter(|order| {
                description
                    .replaced_order_id
                    .as_deref()
                    .is_none_or(|target| {
                        order.broker_order_id != target
                            && order.logical_request_id.as_deref() != Some(target)
                    })
            })
            .try_fold(0_i64, |total, order| {
                let remaining = order.signed_remaining_lots().map_err(|error| {
                    RiskAdmissionError::Unavailable(format!(
                        "broker open-order direction unavailable: {error}"
                    ))
                })?;
                total.checked_add(remaining).ok_or_else(|| {
                    RiskAdmissionError::Unavailable("open-order exposure overflow".into())
                })
            })?;
        let unresolved = self
            .runtime_store
            .unresolved_mutations(&scope.key())
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        let unresolved_unknown_conflict = unresolved.iter().any(|mutation| {
            mutation.state.safety_unresolved()
                && mutation.request_evidence.instrument_ref.as_deref()
                    == Some(description.instrument_id.as_str())
        });
        let active_reservation_delta_lots = self
            .risk_store
            .active_reserved_delta(&self.canonical_account_id, &description.instrument_id)
            .map_err(store_unavailable)?;
        let base_position_lots = current_position_lots
            .checked_add(open_order_delta_lots)
            .and_then(|value| value.checked_add(active_reservation_delta_lots))
            .ok_or_else(|| RiskAdmissionError::Unavailable("risk position overflow".into()))?;
        let increasing_lots = increasing_portion(base_position_lots, description.delta_lots)?;

        let requested_notional_nanos = match description.price {
            Some(price)
                if directional && increasing_lots > 0 && notional_policy_enabled(policy) =>
            {
                let side = if description.delta_lots > 0 {
                    OrderSide::Buy
                } else {
                    OrderSide::Sell
                };
                let total = read_session
                    .risk_reads
                    .limit_order_price(
                        &scope.broker_account_id,
                        &description.instrument_id,
                        price,
                        side,
                        i64::try_from(description.delta_lots.unsigned_abs()).map_err(|_| {
                            RiskAdmissionError::Denied {
                                code: "INVALID_QUANTITY".into(),
                                message: "order quantity is not representable".into(),
                            }
                        })?,
                    )
                    .await
                    .map_err(|error| RiskAdmissionError::Denied {
                        code: "PRICE_UNAVAILABLE".into(),
                        message: error.to_string(),
                    })?
                    .total_order_amount
                    .ok_or_else(|| RiskAdmissionError::Denied {
                        code: "PRICE_UNAVAILABLE".into(),
                        message: "broker limit-order estimate omitted total amount".into(),
                    })?
                    .amount
                    .fixed_point()
                    .total_nanos();
                if description.delta_lots < 0 {
                    total.checked_neg().ok_or_else(|| {
                        RiskAdmissionError::Unavailable("order notional overflow".into())
                    })?
                } else {
                    total
                }
            }
            None if directional && increasing_lots > 0 && notional_policy_enabled(policy) => {
                return Err(RiskAdmissionError::Denied {
                    code: "PRICE_UNAVAILABLE".into(),
                    message: "price-dependent policy cannot validate this order type".into(),
                });
            }
            _ => 0,
        };

        let revision = u64::try_from(cached.report.snapshot_observed_at_unix_ms.max(0))
            .map_err(|_| RiskAdmissionError::Unavailable("snapshot revision invalid".into()))?;
        let instrument_id = description.instrument_id.clone();
        let instrument_id_clone = instrument_id.clone();
        Ok(RiskRequest {
            request_id: logical_request_id.to_owned(),
            account_id: self.canonical_account_id.clone(),
            broker_connection_id: self.broker_connection_id.clone(),
            instrument_id: instrument_id_clone,
            strategy_id: None,
            source: match purpose {
                RuntimeExecutionPurpose::ProductionAutomated => RiskSource::Strategy,
                RuntimeExecutionPurpose::SandboxMutation
                | RuntimeExecutionPurpose::ProductionManual => RiskSource::Manual,
            },
            action: description.action,
            requested_delta_lots: description.delta_lots,
            requested_notional_nanos,
            is_market_order: matches!(description.order_type, Some(RegularOrderType::Market)),
            confirm_margin_trade: description.confirm_margin_trade,
            emergency_reduction: false,
            now_unix_ms: now,
            snapshot: RiskSnapshot {
                runtime_ready: cached.report.resulting_state == RuntimeState::Ready,
                execution_authorized: true,
                instrument_tradable,
                instrument_lot_size: lot_size,
                unresolved_unknown_conflict,
                current_position_lots,
                open_order_delta_lots,
                unresolved_unknown_delta_lots: 0,
                active_reservation_delta_lots,
                gross_exposure_nanos: None,
                net_exposure_nanos: None,
                instrument_exposure_nanos: None,
                broker_daily_pnl_nanos: cached
                    .report
                    .portfolio
                    .broker_daily_yield
                    .as_ref()
                    .map(|value| value.amount_nanos.parse::<i128>())
                    .transpose()
                    .map_err(|error| {
                        RiskAdmissionError::Unavailable(format!(
                            "broker daily_yield is invalid: {error}"
                        ))
                    })?,
                broker_lot_limits,
                margin: if increasing_lots > 0
                    && (policy.max_margin_utilization_ppm.is_some()
                        || (policy.allow_margin && description.confirm_margin_trade))
                {
                    Some(broker_margin_facts(
                        read_session
                            .risk_reads
                            .margin_attributes(&scope.broker_account_id)
                            .await
                            .map_err(|error| RiskAdmissionError::Denied {
                                code: "CRITICAL_INPUT_MISSING".into(),
                                message: error.to_string(),
                            })?,
                        now,
                    )?)
                } else {
                    None
                },
                protection: self
                    .protection_status(&cached.report, &instrument_id)
                    .await?,
                validity: RiskValidityContext {
                    runtime_epoch: cached.report.runtime_epoch,
                    reconciliation_revision: revision,
                    position_revision: revision,
                    order_revision: revision,
                    // GetOrderPrice is not a market-data freshness oracle. Until a shared
                    // quote watermark is attached, price-freshness policies fail closed.
                    market_data_as_of_unix_ms: None,
                    instrument_constraints_revision,
                    policy_revision: policy.revision,
                    execution_authorization_revision: execution.authorization_revision(),
                },
            },
        })
    }
}

#[async_trait]
impl RiskAdmissionPort for ProductionRiskAdapter {
    async fn admit(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
        command: &RuntimeExecutionCommand,
        logical_request_id: &str,
    ) -> Result<RiskAdmission, RiskAdmissionError> {
        if let Some(existing) = self
            .risk_store
            .approval_for_request(&self.canonical_account_id, logical_request_id)
            .map_err(store_unavailable)?
        {
            if !matches!(existing.reservation.state, ReservationState::Active) {
                return Err(RiskAdmissionError::Stale(
                    "logical request reservation is no longer dispatchable".into(),
                ));
            }
            if self
                .runtime_store
                .mutation(&scope.key(), logical_request_id)
                .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?
                .is_some()
            {
                return Err(RiskAdmissionError::Stale(
                    "logical request already entered runtime journal".into(),
                ));
            }
            let cached = self
                .snapshot
                .read()
                .await
                .clone()
                .ok_or_else(|| RiskAdmissionError::Stale("risk snapshot disappeared".into()))?;
            let replay = command_description(command, &cached.report)?;
            if replay.action != existing.decision.action
                || replay.instrument_id != existing.reservation.instrument_id
                || replay.delta_lots != existing.decision.requested_delta_lots
            {
                return Err(RiskAdmissionError::Stale(
                    "logical request replay changed risk semantics".into(),
                ));
            }
            return Ok(RiskAdmission {
                decision_id: existing.decision.decision_id,
                reservation_id: Some(existing.reservation.reservation_id),
                policy_revision: existing.decision.policy_revision,
                approved_delta_lots: existing.decision.approved_delta_lots,
            });
        }
        let policy = self.policy()?;
        let request = self
            .build_request(scope, purpose, command, logical_request_id, &policy)
            .await?;
        let mut decision = RiskEngine::evaluate(&policy, &request)
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        if !decision.permits_dispatch() {
            self.risk_store
                .put_decision(&decision)
                .map_err(store_unavailable)?;
            let reason = decision.reasons.first().cloned().unwrap_or_else(|| {
                vox_risk::RiskReason::new(RiskReasonCode::PersistenceFailure, "risk denied")
            });
            return Err(RiskAdmissionError::Denied {
                code: risk_reason_code(reason.code),
                message: reason.message,
            });
        }

        let reservation = if matches!(
            request.action,
            RiskActionKind::DirectionalOrder | RiskActionKind::ReplaceDirectionalOrder
        ) {
            let reservation_id = RiskReservation::new_id();
            decision.reservation_id = Some(reservation_id.clone());
            let approved_notional = scaled_notional(
                request.requested_notional_nanos,
                request.requested_delta_lots,
                decision.approved_delta_lots,
            )?;
            Some(RiskReservation {
                reservation_id,
                account_id: request.account_id.clone(),
                instrument_id: request.instrument_id.clone(),
                strategy_id: request.strategy_id.clone(),
                source: request.source,
                logical_request_id: request.request_id.clone(),
                reserved_delta_lots: decision.approved_delta_lots,
                remaining_delta_lots: decision.approved_delta_lots,
                reserved_notional_nanos: approved_notional,
                state: ReservationState::Active,
                created_at_unix_ms: request.now_unix_ms,
                updated_at_unix_ms: request.now_unix_ms,
                expires_at_unix_ms: decision.expires_at_unix_ms,
            })
        } else {
            None
        };

        let persisted = if let Some(reservation) = reservation.as_ref() {
            let persisted = self
                .risk_store
                .persist_approval_atomic(
                    &decision,
                    reservation,
                    ReservationCapacity {
                        max_account_reserved_notional_nanos: policy.max_gross_exposure_nanos,
                        max_instrument_reserved_abs_lots: policy.max_position_abs_lots,
                    },
                )
                .map_err(|error| match error {
                    vox_risk::RiskStoreError::CapacityExceeded => RiskAdmissionError::Denied {
                        code: "RESERVATION_CAPACITY_EXCEEDED".into(),
                        message: "concurrent reservations consumed remaining capacity".into(),
                    },
                    other => store_unavailable(other),
                })?;
            (persisted.decision, Some(persisted.reservation))
        } else {
            self.risk_store
                .put_decision(&decision)
                .map_err(store_unavailable)?;
            (decision, None)
        };

        // Create a protection plan when policy mandates protection for new exposure.
        // The plan starts in PLANNED state and is advanced to SUBMITTED once the
        // runtime dispatches the protection leg to the broker.
        if policy.protection_required_for_new_exposure
            && matches!(
                persisted.0.action,
                RiskActionKind::DirectionalOrder | RiskActionKind::ReplaceDirectionalOrder
            )
            && persisted.0.approved_delta_lots != 0
            && let Some(ref reservation) = persisted.1
        {
            let plan = self
                .risk_store
                .create_protection_plan(
                    &self.canonical_account_id,
                    &reservation.instrument_id,
                    reservation.strategy_id.clone(),
                    &reservation.reservation_id,
                    persisted.0.approved_delta_lots,
                    None, // canonical_plan_id wired when runtime dispatches protection leg
                    now_unix_ms()?,
                )
                .map_err(|error| {
                    RiskAdmissionError::Unavailable(format!(
                        "protection plan creation failed: {error}"
                    ))
                })?;
            tracing::info!(
                risk.plan_id = %plan.plan_id,
                risk.reservation_id = %reservation.reservation_id,
                risk.instrument_id = %reservation.instrument_id,
                persistence.event = "protection_plan_created",
                "protection plan created for approved exposure",
            );
        }

        Ok(RiskAdmission {
            decision_id: persisted.0.decision_id,
            reservation_id: persisted.1.map(|reservation| reservation.reservation_id),
            policy_revision: persisted.0.policy_revision,
            approved_delta_lots: persisted.0.approved_delta_lots,
        })
    }

    async fn validate_before_dispatch(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
        command: &RuntimeExecutionCommand,
        logical_request_id: &str,
        admission: &RiskAdmission,
    ) -> Result<(), RiskAdmissionError> {
        let decision = self
            .risk_store
            .decision(&admission.decision_id)
            .map_err(store_unavailable)?
            .ok_or_else(|| RiskAdmissionError::Stale("risk decision disappeared".into()))?;
        if decision.request_id != logical_request_id
            || decision.account_id != self.canonical_account_id
            || decision.policy_revision != admission.policy_revision
            || decision.approved_delta_lots != admission.approved_delta_lots
            || decision.reservation_id != admission.reservation_id
        {
            return Err(RiskAdmissionError::Stale(
                "persisted risk approval does not match dispatch".into(),
            ));
        }
        if let Some(expires_at) = decision.expires_at_unix_ms
            && now_unix_ms()? > expires_at
        {
            return Err(RiskAdmissionError::Stale("risk approval expired".into()));
        }
        if self.policy()?.revision != decision.validity.policy_revision {
            return Err(RiskAdmissionError::Stale(
                "risk policy revision changed".into(),
            ));
        }

        let connection_id = ConnectionId::parse(scope.connection_ref.as_str().to_owned())
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        let execution = self
            .factory
            .execution_session(
                &connection_id,
                &scope.broker_account_id,
                map_purpose(purpose),
            )
            .map_err(|error| RiskAdmissionError::Denied {
                code: "EXECUTION_UNAUTHORIZED".into(),
                message: error.to_string(),
            })?;
        if execution.authorization_revision() != decision.validity.execution_authorization_revision
        {
            return Err(RiskAdmissionError::Stale(
                "execution authorization revision changed".into(),
            ));
        }

        let cached = self
            .snapshot
            .read()
            .await
            .clone()
            .ok_or_else(|| RiskAdmissionError::Stale("risk snapshot disappeared".into()))?;
        let snapshot_revision = u64::try_from(cached.report.snapshot_observed_at_unix_ms.max(0))
            .map_err(|_| RiskAdmissionError::Unavailable("snapshot revision invalid".into()))?;
        if cached.report.runtime_epoch != decision.validity.runtime_epoch
            || snapshot_revision != decision.validity.reconciliation_revision
            || snapshot_revision != decision.validity.position_revision
            || snapshot_revision != decision.validity.order_revision
        {
            return Err(RiskAdmissionError::Stale(
                "broker account-state watermark changed".into(),
            ));
        }
        let description = command_description(command, &cached.report)?;
        if description.delta_lots != decision.approved_delta_lots
            || description.action != decision.action
        {
            return Err(RiskAdmissionError::Stale(
                "approved command semantics changed".into(),
            ));
        }
        if matches!(
            description.action,
            RiskActionKind::DirectionalOrder | RiskActionKind::ReplaceDirectionalOrder
        ) {
            let cache = self.instrument_cache.lock().await;
            let current_revision = cache
                .get(&description.instrument_id)
                .map(|(cached_at, _)| risk_revision(*cached_at))
                .transpose()?
                .ok_or_else(|| {
                    RiskAdmissionError::Stale("instrument constraints disappeared".into())
                })?;
            if current_revision != decision.validity.instrument_constraints_revision {
                return Err(RiskAdmissionError::Stale(
                    "instrument constraint revision changed".into(),
                ));
            }
        }
        let unresolved = self
            .runtime_store
            .unresolved_mutations(&scope.key())
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        if unresolved.iter().any(|mutation| {
            mutation.logical_request_id != logical_request_id
                && mutation.state.safety_unresolved()
                && mutation.request_evidence.instrument_ref.as_deref()
                    == Some(description.instrument_id.as_str())
        }) {
            return Err(RiskAdmissionError::Stale(
                "conflicting UNKNOWN_AFTER_DISPATCH mutation appeared".into(),
            ));
        }
        Ok(())
    }

    async fn record_dispatch_outcome(
        &self,
        _scope: &RuntimeScope,
        logical_request_id: &str,
        outcome: RiskDispatchOutcome,
    ) -> Result<(), RiskAdmissionError> {
        let reconciler = RiskReservationReconciler::new(self.risk_store.clone());
        let now = now_unix_ms()?;
        let result = match outcome {
            RiskDispatchOutcome::Acknowledged => {
                reconciler.dispatch_acknowledged(&self.canonical_account_id, logical_request_id)
            }
            RiskDispatchOutcome::Rejected => reconciler.broker_authoritative_reject(
                &self.canonical_account_id,
                logical_request_id,
                now,
            ),
            RiskDispatchOutcome::UnknownAfterDispatch => reconciler.unknown_after_dispatch(
                &self.canonical_account_id,
                logical_request_id,
                now,
            ),
        };
        match result {
            Ok(_) | Err(vox_risk::RiskReservationReconcileError::ReservationNotFound) => Ok(()),
            Err(error) => Err(RiskAdmissionError::Unavailable(error.to_string())),
        }
    }

    async fn reconcile(
        &self,
        _scope: &RuntimeScope,
        report: &ReconciliationReport,
    ) -> Result<(), RiskAdmissionError> {
        *self.snapshot.write().await = Some(ProductionRiskSnapshot {
            report: report.clone(),
        });
        let reconciler = RiskReservationReconciler::new(self.risk_store.clone());
        let active = self.active_reservations()?;
        for reservation in active {
            if report
                .unresolved_logical_request_ids
                .contains(&reservation.logical_request_id)
            {
                reconciler
                    .unknown_after_dispatch(
                        &self.canonical_account_id,
                        &reservation.logical_request_id,
                        report.completed_at_unix_ms,
                    )
                    .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
                continue;
            }
            if let Some(order) = report.active_orders.iter().find(|order| {
                order.logical_request_id.as_deref() == Some(&reservation.logical_request_id)
            }) {
                reconciler
                    .runtime_reconciliation(
                        &self.canonical_account_id,
                        &reservation.logical_request_id,
                        order
                            .signed_remaining_lots()
                            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?,
                        report.completed_at_unix_ms,
                    )
                    .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
            } else {
                reconciler
                    .broker_authoritative_no_remaining_exposure(
                        &self.canonical_account_id,
                        &reservation.logical_request_id,
                        report.completed_at_unix_ms,
                    )
                    .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
            }
        }

        // Reconcile protection plans against broker stop facts.
        // This restores correlation after restart and transitions plans based
        // on broker-authoritative stop evidence (active / cancelled / expired).
        for position in &report.positions {
            let active_stop_lots: i64 = report
                .active_stops
                .iter()
                .filter(|stop| {
                    stop.instrument_uid == position.instrument_uid && stop.status.active()
                })
                .map(|stop| stop.quantity_lots.unwrap_or(0))
                .sum();

            self.risk_store
                .reconcile_protection_plans(
                    &self.canonical_account_id,
                    &position.instrument_uid,
                    active_stop_lots,
                    position.quantity_units,
                    report.completed_at_unix_ms,
                )
                .map_err(|error| {
                    RiskAdmissionError::Unavailable(format!(
                        "protection plan reconciliation failed: {error}"
                    ))
                })?;
        }

        Ok(())
    }

    async fn transition_protection_plan_on_dispatch(
        &self,
        _scope: &RuntimeScope,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<(), RiskAdmissionError> {
        // Find the protection plan linked to this logical request via its reservation.
        let reservation = self
            .risk_store
            .reservation_for_request(&self.canonical_account_id, logical_request_id)
            .map_err(store_unavailable)?
            .ok_or_else(|| {
                RiskAdmissionError::Unavailable(
                    "no reservation found for protection plan transition".into(),
                )
            })?;

        // Find the protection plan linked to this reservation.
        let plans = self
            .risk_store
            .protection_plans_for_instrument(&self.canonical_account_id, &reservation.instrument_id)
            .map_err(store_unavailable)?;

        let plan = plans
            .iter()
            .find(|p| p.reservation_id == reservation.reservation_id)
            .ok_or_else(|| {
                RiskAdmissionError::Unavailable("no protection plan found for reservation".into())
            })?;

        // Transition from Planned to Submitted.
        self.risk_store
            .transition_protection_plan(
                &plan.plan_id,
                &[ProtectionPlanState::Planned],
                ProtectionPlanState::Submitted,
                now_unix_ms,
            )
            .map_err(|error| match error {
                vox_risk::RiskStoreError::InvalidTransition => RiskAdmissionError::Unavailable(
                    "protection plan is not in Planned state".into(),
                ),
                other => store_unavailable(other),
            })?;

        tracing::info!(
            risk.plan_id = %plan.plan_id,
            risk.reservation_id = %reservation.reservation_id,
            persistence.event = "protection_plan_submitted",
            "protection plan transitioned to SUBMITTED",
        );
        Ok(())
    }

    async fn transition_protection_plan_on_reject(
        &self,
        _scope: &RuntimeScope,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<(), RiskAdmissionError> {
        // Find the protection plan linked to this logical request via its reservation.
        let reservation = self
            .risk_store
            .reservation_for_request(&self.canonical_account_id, logical_request_id)
            .map_err(store_unavailable)?
            .ok_or_else(|| {
                RiskAdmissionError::Unavailable(
                    "no reservation found for protection plan reject transition".into(),
                )
            })?;

        // Find the protection plan linked to this reservation.
        let plans = self
            .risk_store
            .protection_plans_for_instrument(&self.canonical_account_id, &reservation.instrument_id)
            .map_err(store_unavailable)?;

        let plan = plans
            .into_iter()
            .find(|p| p.reservation_id == reservation.reservation_id)
            .ok_or_else(|| {
                RiskAdmissionError::Unavailable("no protection plan found for reservation".into())
            })?;

        // Transition from Planned to Failed.
        self.risk_store
            .transition_protection_plan(
                &plan.plan_id,
                &[ProtectionPlanState::Planned],
                ProtectionPlanState::Failed,
                now_unix_ms,
            )
            .map_err(|error| match error {
                vox_risk::RiskStoreError::InvalidTransition => RiskAdmissionError::Unavailable(
                    "protection plan is not in Planned state".into(),
                ),
                other => store_unavailable(other),
            })?;

        tracing::info!(
            risk.plan_id = %plan.plan_id,
            risk.reservation_id = %reservation.reservation_id,
            persistence.event = "protection_plan_failed",
            "protection plan transitioned to FAILED",
        );
        Ok(())
    }
}

struct CommandDescription {
    action: RiskActionKind,
    instrument_id: String,
    delta_lots: i64,
    price: Option<vox_domain::FixedPoint>,
    order_type: Option<RegularOrderType>,
    confirm_margin_trade: bool,
    replaced_order_id: Option<String>,
}

fn command_description(
    command: &RuntimeExecutionCommand,
    report: &ReconciliationReport,
) -> Result<CommandDescription, RiskAdmissionError> {
    match command {
        RuntimeExecutionCommand::RegularOrder(order)
        | RuntimeExecutionCommand::PostOrderAsync(order) => Ok(CommandDescription {
            action: RiskActionKind::DirectionalOrder,
            instrument_id: order.instrument_id.clone(),
            delta_lots: signed_lots(order.side, order.quantity_lots)?,
            price: order.price,
            order_type: Some(order.order_type),
            confirm_margin_trade: order.confirm_margin_trade,
            replaced_order_id: None,
        }),
        RuntimeExecutionCommand::ReplaceOrder(order) => {
            let existing = report
                .active_orders
                .iter()
                .find(|candidate| {
                    candidate.broker_order_id == order.existing_order_id
                        || candidate.logical_request_id.as_deref()
                            == Some(order.existing_order_id.as_str())
                })
                .ok_or_else(|| RiskAdmissionError::Denied {
                    code: "ORDER_NOT_FOUND".into(),
                    message: "replacement target absent from authoritative open orders".into(),
                })?;
            let side = existing.side.ok_or_else(|| RiskAdmissionError::Denied {
                code: "ORDER_DIRECTION_UNKNOWN".into(),
                message: "replacement target direction is unavailable".into(),
            })?;
            Ok(CommandDescription {
                action: RiskActionKind::ReplaceDirectionalOrder,
                instrument_id: existing.instrument_uid.clone(),
                delta_lots: signed_lots(side, order.quantity_lots)?,
                price: Some(order.price),
                order_type: Some(RegularOrderType::Limit),
                confirm_margin_trade: order.confirm_margin_trade,
                replaced_order_id: Some(order.existing_order_id.clone()),
            })
        }
        RuntimeExecutionCommand::CancelOrder(order) => Ok(CommandDescription {
            action: RiskActionKind::CancelOrder,
            instrument_id: report
                .active_orders
                .iter()
                .find(|candidate| {
                    candidate.broker_order_id == order.order_id
                        || candidate.logical_request_id.as_deref() == Some(order.order_id.as_str())
                })
                .map_or_else(
                    || "maintenance:cancel-order".into(),
                    |value| value.instrument_uid.clone(),
                ),
            delta_lots: 0,
            price: None,
            order_type: None,
            confirm_margin_trade: false,
            replaced_order_id: None,
        }),
        RuntimeExecutionCommand::PostStopOrder(order)
        | RuntimeExecutionCommand::ProtectionLeg(order) => Ok(CommandDescription {
            action: RiskActionKind::ProtectionMaintenance,
            instrument_id: order.instrument_id.clone(),
            delta_lots: 0,
            price: None,
            order_type: None,
            confirm_margin_trade: order.confirm_margin_trade,
            replaced_order_id: None,
        }),
        RuntimeExecutionCommand::CancelStopOrder(_) => Ok(CommandDescription {
            action: RiskActionKind::CancelProtection,
            instrument_id: "maintenance:cancel-protection".into(),
            delta_lots: 0,
            price: None,
            order_type: None,
            confirm_margin_trade: false,
            replaced_order_id: None,
        }),
    }
}

fn signed_lots(side: OrderSide, lots: i64) -> Result<i64, RiskAdmissionError> {
    if lots <= 0 {
        return Err(RiskAdmissionError::Denied {
            code: "INVALID_QUANTITY".into(),
            message: "quantity must be positive".into(),
        });
    }
    match side {
        OrderSide::Buy => Ok(lots),
        OrderSide::Sell => lots
            .checked_neg()
            .ok_or_else(|| RiskAdmissionError::Denied {
                code: "INVALID_QUANTITY".into(),
                message: "quantity is not representable".into(),
            }),
    }
}

fn increasing_portion(base: i64, delta: i64) -> Result<i64, RiskAdmissionError> {
    if delta == 0 {
        return Ok(0);
    }
    if base == 0 || base.signum() == delta.signum() {
        return i64::try_from(delta.unsigned_abs())
            .map_err(|_| RiskAdmissionError::Unavailable("risk quantity overflow".into()));
    }
    let base_abs = i64::try_from(base.unsigned_abs())
        .map_err(|_| RiskAdmissionError::Unavailable("risk quantity overflow".into()))?;
    let delta_abs = i64::try_from(delta.unsigned_abs())
        .map_err(|_| RiskAdmissionError::Unavailable("risk quantity overflow".into()))?;
    Ok(delta_abs.saturating_sub(base_abs))
}

fn broker_margin_facts(
    value: vox_tinvest::risk_read::CanonicalMarginAttributes,
    broker_as_of_unix_ms: i64,
) -> Result<BrokerMarginFacts, RiskAdmissionError> {
    let liquid = required_margin_money(value.liquid_portfolio, "liquid_portfolio")?;
    let starting = required_margin_money(value.starting_margin, "starting_margin")?;
    let minimal = required_margin_money(value.minimal_margin, "minimal_margin")?;
    let corrected = required_margin_money(value.corrected_margin, "corrected_margin")?;
    let missing = required_margin_money(value.amount_of_missing_funds, "amount_of_missing_funds")?;
    let futures = required_margin_money(value.guarantee_for_futures, "guarantee_for_futures")?;
    if liquid.0 != corrected.0 {
        return Err(RiskAdmissionError::Denied {
            code: "CRITICAL_INPUT_MISSING".into(),
            message: "broker margin currencies are inconsistent".into(),
        });
    }
    let funds_sufficiency_ppm = value
        .funds_sufficiency_level
        .map(|level| {
            i64::try_from(level.fixed_point().total_nanos() / 1_000).map_err(|_| {
                RiskAdmissionError::Denied {
                    code: "CRITICAL_INPUT_MISSING".into(),
                    message: "funds sufficiency level is out of range".into(),
                }
            })
        })
        .transpose()?;
    Ok(BrokerMarginFacts {
        liquid_portfolio_nanos: liquid.1,
        starting_margin_nanos: starting.1,
        minimal_margin_nanos: minimal.1,
        corrected_margin_nanos: corrected.1,
        funds_sufficiency_ppm,
        amount_of_missing_funds_nanos: missing.1,
        guarantee_for_futures_nanos: futures.1,
        broker_as_of_unix_ms,
    })
}

fn required_margin_money(
    value: Option<vox_tinvest::canonical::CanonicalMoney>,
    field: &'static str,
) -> Result<(Option<String>, i128), RiskAdmissionError> {
    value
        .map(|money| (money.currency, money.amount.fixed_point().total_nanos()))
        .ok_or_else(|| RiskAdmissionError::Denied {
            code: "CRITICAL_INPUT_MISSING".into(),
            message: format!("broker margin field {field} is missing"),
        })
}

fn map_lot_limits(value: vox_tinvest::execution::CanonicalMaxLots) -> BrokerLotLimits {
    BrokerLotLimits {
        buy_own: value.buy.map(|limit| BuyLotLimit {
            max_lots: limit.max_lots,
            max_market_lots: limit.max_market_lots,
        }),
        buy_margin: value.buy_with_margin.map(|limit| BuyLotLimit {
            max_lots: limit.max_lots,
            max_market_lots: limit.max_market_lots,
        }),
        sell_own: value.sell.map(|limit| SellLotLimit {
            max_lots: limit.max_lots,
        }),
        sell_margin: value.sell_with_margin.map(|limit| SellLotLimit {
            max_lots: limit.max_lots,
        }),
    }
}

fn initial_policy() -> RiskPolicySet {
    RiskPolicySet {
        revision: 1,
        state: RiskState::ReduceOnly,
        allow_margin: false,
        require_provider_lot_limit: true,
        max_market_data_age_ms: None,
        max_single_order_lots: None,
        max_position_abs_lots: None,
        max_gross_exposure_nanos: None,
        max_net_exposure_abs_nanos: None,
        max_instrument_exposure_nanos: None,
        max_margin_utilization_ppm: None,
        max_daily_loss_nanos: None,
        protection_required_for_new_exposure: false,
        max_unprotected_duration_ms: None,
    }
}

fn notional_policy_enabled(policy: &RiskPolicySet) -> bool {
    policy.max_gross_exposure_nanos.is_some()
        || policy.max_net_exposure_abs_nanos.is_some()
        || policy.max_instrument_exposure_nanos.is_some()
}

fn scaled_notional(
    requested_notional: i128,
    requested_delta: i64,
    approved_delta: i64,
) -> Result<i128, RiskAdmissionError> {
    let requested_abs = i128::from(requested_delta)
        .checked_abs()
        .ok_or_else(|| RiskAdmissionError::Unavailable("requested quantity overflow".into()))?;
    let approved_abs = i128::from(approved_delta)
        .checked_abs()
        .ok_or_else(|| RiskAdmissionError::Unavailable("approved quantity overflow".into()))?;
    if requested_abs == 0 {
        return Ok(0);
    }
    requested_notional
        .checked_mul(approved_abs)
        .and_then(|value| value.checked_div(requested_abs))
        .ok_or_else(|| RiskAdmissionError::Unavailable("approved notional overflow".into()))
}

fn map_purpose(value: RuntimeExecutionPurpose) -> ExecutionPurpose {
    match value {
        RuntimeExecutionPurpose::SandboxMutation => ExecutionPurpose::SandboxMutation,
        RuntimeExecutionPurpose::ProductionManual => ExecutionPurpose::ProductionManual,
        RuntimeExecutionPurpose::ProductionAutomated => ExecutionPurpose::ProductionAutomated,
    }
}

fn now_unix_ms() -> Result<i64, RiskAdmissionError> {
    let nanos = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000)
        .map_err(|_| RiskAdmissionError::Unavailable("system clock is out of range".into()))
}

fn risk_revision(unix_ms: i64) -> Result<u64, RiskAdmissionError> {
    u64::try_from(unix_ms)
        .map_err(|_| RiskAdmissionError::Unavailable("risk revision is out of range".into()))
}

fn store_unavailable(error: vox_risk::RiskStoreError) -> RiskAdmissionError {
    RiskAdmissionError::Unavailable(error.to_string())
}

fn risk_reason_code(code: RiskReasonCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "PERSISTENCE_FAILURE".into())
}
