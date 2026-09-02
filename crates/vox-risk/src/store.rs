use std::path::{Path, PathBuf};
use std::time::Instant;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;

use crate::model::{ReservationState, RiskDecision, RiskPolicySet, RiskReservation, RiskSource};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReservationCapacity {
    pub max_account_reserved_notional_nanos: Option<i128>,
    pub max_instrument_reserved_abs_lots: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedRiskApproval {
    pub decision: RiskDecision,
    pub reservation: RiskReservation,
}

pub trait RiskStore: Clone + Send + Sync + 'static {
    fn policy(&self, account_id: &str) -> Result<Option<RiskPolicySet>, RiskStoreError>;
    fn put_policy(
        &self,
        account_id: &str,
        expected_revision: Option<u64>,
        policy: &RiskPolicySet,
        reason: &str,
        now_unix_ms: i64,
    ) -> Result<RiskPolicySet, RiskStoreError>;
    fn put_decision(&self, decision: &RiskDecision) -> Result<(), RiskStoreError>;
    fn decision(&self, decision_id: &str) -> Result<Option<RiskDecision>, RiskStoreError>;
    fn decision_for_request(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<Option<RiskDecision>, RiskStoreError>;
    fn approval_for_request(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<Option<PersistedRiskApproval>, RiskStoreError>;
    fn persist_approval_atomic(
        &self,
        decision: &RiskDecision,
        reservation: &RiskReservation,
        capacity: ReservationCapacity,
    ) -> Result<PersistedRiskApproval, RiskStoreError>;
    fn reserve_atomic(
        &self,
        reservation: &RiskReservation,
        max_active_abs_lots: i64,
    ) -> Result<RiskReservation, RiskStoreError>;
    fn reservation_for_request(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<Option<RiskReservation>, RiskStoreError>;
    fn active_reserved_delta(
        &self,
        account_id: &str,
        instrument_id: &str,
    ) -> Result<i64, RiskStoreError>;
    fn active_reservations(&self, account_id: &str)
    -> Result<Vec<RiskReservation>, RiskStoreError>;
    fn update_reservation(
        &self,
        reservation_id: &str,
        expected: &[ReservationState],
        remaining_delta_lots: i64,
        state: ReservationState,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskStoreError>;
}

#[derive(Clone)]
pub struct SqliteRiskStore {
    path: PathBuf,
}

impl SqliteRiskStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RiskStoreError> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        store.connection()?.execute_batch(SCHEMA)?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, RiskStoreError> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }
}

impl RiskStore for SqliteRiskStore {
    fn policy(&self, account_id: &str) -> Result<Option<RiskPolicySet>, RiskStoreError> {
        self.connection()?
            .query_row(
                "SELECT payload FROM risk_policies WHERE account_id = ?1",
                [account_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    fn put_policy(
        &self,
        account_id: &str,
        expected_revision: Option<u64>,
        policy: &RiskPolicySet,
        reason: &str,
        now_unix_ms: i64,
    ) -> Result<RiskPolicySet, RiskStoreError> {
        if account_id.trim().is_empty() || reason.trim().is_empty() || policy.revision == 0 {
            return Err(RiskStoreError::InvalidPolicyMutation);
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT revision, payload FROM risk_policies WHERE account_id = ?1",
                [account_id],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match (expected_revision, current.as_ref()) {
            (None, None) => {}
            (Some(expected), Some((actual, _))) if expected == *actual => {}
            _ => return Err(RiskStoreError::PolicyRevisionConflict),
        }
        if let Some((actual, _)) = current.as_ref()
            && policy.revision <= *actual
        {
            return Err(RiskStoreError::PolicyRevisionConflict);
        }
        let payload = serde_json::to_string(policy)?;
        transaction.execute(
            "INSERT INTO risk_policies(account_id, revision, payload, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id) DO UPDATE SET
               revision = excluded.revision,
               payload = excluded.payload,
               updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![account_id, policy.revision, payload, now_unix_ms],
        )?;
        transaction.execute(
            "INSERT INTO risk_policy_audit(
                event_id, account_id, old_revision, new_revision, reason, payload,
                observed_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                format!("risk-policy-audit:{}", uuid::Uuid::new_v4()),
                account_id,
                current.as_ref().map(|(revision, _)| *revision),
                policy.revision,
                reason,
                serde_json::to_string(policy)?,
                now_unix_ms,
            ],
        )?;
        transaction.commit()?;
        Ok(policy.clone())
    }

    fn put_decision(&self, decision: &RiskDecision) -> Result<(), RiskStoreError> {
        let start = Instant::now();
        let payload = serde_json::to_string(decision)?;
        self.connection()?.execute(
            "INSERT INTO risk_decisions(decision_id, request_id, account_id, policy_revision, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(decision_id) DO NOTHING",
            params![
                decision.decision_id,
                decision.request_id,
                decision.account_id,
                decision.policy_revision,
                payload
            ],
        )?;
        tracing::debug!(
            risk.decision_id = %decision.decision_id,
            risk.request_id = %decision.request_id,
            risk.account_id = %decision.account_id,
            risk.outcome = ?decision.outcome,
            persistence.elapsed_ms = start.elapsed().as_millis(),
            "risk decision persisted",
        );
        Ok(())
    }

    fn decision(&self, decision_id: &str) -> Result<Option<RiskDecision>, RiskStoreError> {
        decision_by_id_connection(&self.connection()?, decision_id)
    }

    fn decision_for_request(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<Option<RiskDecision>, RiskStoreError> {
        decision_for_request_connection(&self.connection()?, account_id, logical_request_id)
    }

    fn approval_for_request(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<Option<PersistedRiskApproval>, RiskStoreError> {
        let connection = self.connection()?;
        let reservation =
            reservation_for_request_connection(&connection, account_id, logical_request_id)?;
        let decision =
            decision_for_request_connection(&connection, account_id, logical_request_id)?;
        match (decision, reservation) {
            (None, None) => Ok(None),
            (Some(decision), Some(reservation)) => {
                validate_approval_link(&decision, &reservation)?;
                Ok(Some(PersistedRiskApproval {
                    decision,
                    reservation,
                }))
            }
            _ => Err(RiskStoreError::ApprovalInvariantViolation(
                "risk request has incomplete persisted approval",
            )),
        }
    }

    fn persist_approval_atomic(
        &self,
        decision: &RiskDecision,
        reservation: &RiskReservation,
        capacity: ReservationCapacity,
    ) -> Result<PersistedRiskApproval, RiskStoreError> {
        let start = Instant::now();
        validate_approval_link(decision, reservation)?;
        validate_capacity(capacity)?;

        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing_reservation) = reservation_for_request_tx(
            &transaction,
            &reservation.account_id,
            &reservation.logical_request_id,
        )? {
            let existing_decision = decision_for_request_connection(
                &transaction,
                &decision.account_id,
                &decision.request_id,
            )?
            .ok_or(RiskStoreError::ApprovalInvariantViolation(
                "reservation exists without its risk decision",
            ))?;
            transaction.commit()?;
            tracing::debug!(
                risk.decision_id = %existing_decision.decision_id,
                risk.reservation_id = %existing_reservation.reservation_id,
                risk.account_id = %decision.account_id,
                risk.request_id = %decision.request_id,
                persistence.event = "idempotent_approval",
                persistence.elapsed_ms = start.elapsed().as_millis(),
                "risk approval persisted (idempotent replay)",
            );
            return Ok(PersistedRiskApproval {
                decision: existing_decision,
                reservation: existing_reservation,
            });
        }

        enforce_capacity(&transaction, reservation, capacity)?;
        insert_decision(&transaction, decision)?;
        insert_reservation(&transaction, reservation)?;
        transaction.commit()?;

        tracing::info!(
            risk.decision_id = %decision.decision_id,
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %decision.account_id,
            risk.request_id = %decision.request_id,
            risk.instrument_id = %reservation.instrument_id,
            risk.reserved_delta_lots = reservation.reserved_delta_lots,
            persistence.event = "approval_reserved",
            persistence.elapsed_ms = start.elapsed().as_millis(),
            "risk approval persisted with reservation",
        );

        Ok(PersistedRiskApproval {
            decision: decision.clone(),
            reservation: reservation.clone(),
        })
    }

    fn reserve_atomic(
        &self,
        reservation: &RiskReservation,
        max_active_abs_lots: i64,
    ) -> Result<RiskReservation, RiskStoreError> {
        let start = Instant::now();
        if max_active_abs_lots < 0 {
            return Err(RiskStoreError::InvalidCapacity);
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = reservation_for_request_tx(
            &transaction,
            &reservation.account_id,
            &reservation.logical_request_id,
        )? {
            transaction.commit()?;
            tracing::debug!(
                risk.reservation_id = %existing.reservation_id,
                risk.account_id = %reservation.account_id,
                risk.request_id = %reservation.logical_request_id,
                persistence.event = "idempotent_reserve",
                persistence.elapsed_ms = start.elapsed().as_millis(),
                "risk reservation created (idempotent replay)",
            );
            return Ok(existing);
        }

        let active: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(ABS(remaining_delta_lots)), 0)
             FROM risk_reservations
             WHERE account_id = ?1
               AND state IN ('ACTIVE','PARTIALLY_CONSUMED','UNKNOWN_HELD','ORPHANED')",
            [&reservation.account_id],
            |row| row.get(0),
        )?;
        let requested = i64::try_from(reservation.remaining_delta_lots.unsigned_abs())
            .map_err(|_| RiskStoreError::ArithmeticOverflow)?;
        let projected = active
            .checked_add(requested)
            .ok_or(RiskStoreError::ArithmeticOverflow)?;
        if projected > max_active_abs_lots {
            tracing::info!(
                risk.account_id = %reservation.account_id,
                risk.instrument_id = %reservation.instrument_id,
                risk.request_id = %reservation.logical_request_id,
                persistence.event = "capacity_exceeded",
                persistence.active_lots = active,
                persistence.requested_lots = requested,
                persistence.max_lots = max_active_abs_lots,
                "risk reservation capacity exceeded",
            );
            return Err(RiskStoreError::CapacityExceeded);
        }

        insert_reservation(&transaction, reservation)?;
        transaction.commit()?;
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %reservation.account_id,
            risk.instrument_id = %reservation.instrument_id,
            risk.request_id = %reservation.logical_request_id,
            risk.reserved_delta_lots = reservation.reserved_delta_lots,
            persistence.event = "reservation_created",
            persistence.active_lots_after = projected,
            persistence.max_lots = max_active_abs_lots,
            persistence.elapsed_ms = start.elapsed().as_millis(),
            "risk reservation created",
        );
        Ok(reservation.clone())
    }

    fn reservation_for_request(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<Option<RiskReservation>, RiskStoreError> {
        reservation_for_request_connection(&self.connection()?, account_id, logical_request_id)
    }

    fn active_reserved_delta(
        &self,
        account_id: &str,
        instrument_id: &str,
    ) -> Result<i64, RiskStoreError> {
        self.connection()?
            .query_row(
                "SELECT COALESCE(SUM(remaining_delta_lots), 0)
                 FROM risk_reservations
                 WHERE account_id = ?1 AND instrument_id = ?2
                   AND state IN ('ACTIVE','PARTIALLY_CONSUMED','UNKNOWN_HELD','ORPHANED')",
                params![account_id, instrument_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    fn active_reservations(
        &self,
        account_id: &str,
    ) -> Result<Vec<RiskReservation>, RiskStoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT reservation_id, account_id, instrument_id, strategy_id, source,
                    logical_request_id, reserved_delta_lots, remaining_delta_lots,
                    reserved_notional_nanos, state, created_at_unix_ms, updated_at_unix_ms,
                    expires_at_unix_ms
             FROM risk_reservations
             WHERE account_id = ?1
               AND state IN ('ACTIVE','PARTIALLY_CONSUMED','UNKNOWN_HELD','ORPHANED')
             ORDER BY created_at_unix_ms, reservation_id",
        )?;
        statement
            .query_map([account_id], read_reservation)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn update_reservation(
        &self,
        reservation_id: &str,
        expected: &[ReservationState],
        remaining_delta_lots: i64,
        state: ReservationState,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskStoreError> {
        let start = Instant::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = reservation_by_id(&transaction, reservation_id)?
            .ok_or(RiskStoreError::ReservationNotFound)?;
        if !expected.contains(&current.state) {
            return Err(RiskStoreError::InvalidTransition);
        }
        transaction.execute(
            "UPDATE risk_reservations
             SET remaining_delta_lots = ?2, state = ?3, updated_at_unix_ms = ?4
             WHERE reservation_id = ?1",
            params![
                reservation_id,
                remaining_delta_lots,
                state_name(state),
                now_unix_ms
            ],
        )?;
        let updated = reservation_by_id(&transaction, reservation_id)?
            .ok_or(RiskStoreError::ReservationNotFound)?;
        transaction.commit()?;
        tracing::info!(
            risk.reservation_id = %reservation_id,
            risk.account_id = %updated.account_id,
            persistence.event = "reservation_state_transition",
            persistence.from_state = ?current.state,
            persistence.to_state = ?state,
            persistence.remaining_delta_lots = remaining_delta_lots,
            persistence.elapsed_ms = start.elapsed().as_millis(),
            "risk reservation state transition: {:?} -> {:?}",
            current.state,
            state,
        );
        Ok(updated)
    }
}

fn validate_approval_link(
    decision: &RiskDecision,
    reservation: &RiskReservation,
) -> Result<(), RiskStoreError> {
    if decision.account_id != reservation.account_id
        || decision.request_id != reservation.logical_request_id
        || decision.reservation_id.as_deref() != Some(reservation.reservation_id.as_str())
        || !decision.permits_dispatch()
        || decision.approved_delta_lots != reservation.reserved_delta_lots
    {
        return Err(RiskStoreError::ApprovalInvariantViolation(
            "decision and reservation do not describe the same approved request",
        ));
    }
    Ok(())
}

fn validate_capacity(capacity: ReservationCapacity) -> Result<(), RiskStoreError> {
    if capacity
        .max_account_reserved_notional_nanos
        .is_some_and(|value| value < 0)
        || capacity
            .max_instrument_reserved_abs_lots
            .is_some_and(|value| value < 0)
    {
        return Err(RiskStoreError::InvalidCapacity);
    }
    Ok(())
}

fn enforce_capacity(
    connection: &Connection,
    reservation: &RiskReservation,
    capacity: ReservationCapacity,
) -> Result<(), RiskStoreError> {
    if let Some(limit) = capacity.max_instrument_reserved_abs_lots {
        let mut statement = connection.prepare(
            "SELECT remaining_delta_lots
             FROM risk_reservations
             WHERE account_id = ?1 AND instrument_id = ?2
               AND state IN ('ACTIVE','PARTIALLY_CONSUMED','UNKNOWN_HELD','ORPHANED')",
        )?;
        let values = statement.query_map(
            params![reservation.account_id, reservation.instrument_id],
            |row| row.get::<_, i64>(0),
        )?;
        let mut active = 0_i64;
        for value in values {
            let value = value?;
            let absolute = i64::try_from(value.unsigned_abs())
                .map_err(|_| RiskStoreError::ArithmeticOverflow)?;
            active = active
                .checked_add(absolute)
                .ok_or(RiskStoreError::ArithmeticOverflow)?;
        }
        let requested = i64::try_from(reservation.remaining_delta_lots.unsigned_abs())
            .map_err(|_| RiskStoreError::ArithmeticOverflow)?;
        if active
            .checked_add(requested)
            .ok_or(RiskStoreError::ArithmeticOverflow)?
            > limit
        {
            return Err(RiskStoreError::CapacityExceeded);
        }
    }

    if let Some(limit) = capacity.max_account_reserved_notional_nanos {
        let mut statement = connection.prepare(
            "SELECT reserved_notional_nanos
             FROM risk_reservations
             WHERE account_id = ?1
               AND state IN ('ACTIVE','PARTIALLY_CONSUMED','UNKNOWN_HELD','ORPHANED')",
        )?;
        let values =
            statement.query_map([&reservation.account_id], |row| row.get::<_, String>(0))?;
        let mut active = 0_i128;
        for value in values {
            let value = value?
                .parse::<i128>()
                .map_err(|error| RiskStoreError::StoredNumeric(error.to_string()))?;
            active = active
                .checked_add(
                    value
                        .checked_abs()
                        .ok_or(RiskStoreError::ArithmeticOverflow)?,
                )
                .ok_or(RiskStoreError::ArithmeticOverflow)?;
        }
        let requested = reservation
            .reserved_notional_nanos
            .checked_abs()
            .ok_or(RiskStoreError::ArithmeticOverflow)?;
        if active
            .checked_add(requested)
            .ok_or(RiskStoreError::ArithmeticOverflow)?
            > limit
        {
            return Err(RiskStoreError::CapacityExceeded);
        }
    }

    Ok(())
}

fn insert_decision(connection: &Connection, decision: &RiskDecision) -> Result<(), RiskStoreError> {
    let payload = serde_json::to_string(decision)?;
    connection.execute(
        "INSERT INTO risk_decisions(decision_id, request_id, account_id, policy_revision, payload)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            decision.decision_id,
            decision.request_id,
            decision.account_id,
            decision.policy_revision,
            payload
        ],
    )?;
    Ok(())
}

fn decision_by_id_connection(
    connection: &Connection,
    decision_id: &str,
) -> Result<Option<RiskDecision>, RiskStoreError> {
    connection
        .query_row(
            "SELECT payload FROM risk_decisions WHERE decision_id = ?1",
            [decision_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
        .transpose()
}

fn decision_for_request_connection(
    connection: &Connection,
    account_id: &str,
    request_id: &str,
) -> Result<Option<RiskDecision>, RiskStoreError> {
    connection
        .query_row(
            "SELECT payload FROM risk_decisions WHERE account_id = ?1 AND request_id = ?2",
            params![account_id, request_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
        .transpose()
}

fn insert_reservation(
    connection: &Connection,
    reservation: &RiskReservation,
) -> Result<(), RiskStoreError> {
    connection.execute(
        "INSERT INTO risk_reservations(
            reservation_id, account_id, instrument_id, strategy_id, source,
            logical_request_id, reserved_delta_lots, remaining_delta_lots,
            reserved_notional_nanos, state, created_at_unix_ms, updated_at_unix_ms,
            expires_at_unix_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            reservation.reservation_id,
            reservation.account_id,
            reservation.instrument_id,
            reservation.strategy_id,
            source_name(reservation.source),
            reservation.logical_request_id,
            reservation.reserved_delta_lots,
            reservation.remaining_delta_lots,
            reservation.reserved_notional_nanos.to_string(),
            state_name(reservation.state),
            reservation.created_at_unix_ms,
            reservation.updated_at_unix_ms,
            reservation.expires_at_unix_ms,
        ],
    )?;
    Ok(())
}

fn reservation_for_request_connection(
    connection: &Connection,
    account_id: &str,
    logical_request_id: &str,
) -> Result<Option<RiskReservation>, RiskStoreError> {
    connection
        .query_row(
            "SELECT reservation_id, account_id, instrument_id, strategy_id, source,
                    logical_request_id, reserved_delta_lots, remaining_delta_lots,
                    reserved_notional_nanos, state, created_at_unix_ms, updated_at_unix_ms,
                    expires_at_unix_ms
             FROM risk_reservations
             WHERE account_id = ?1 AND logical_request_id = ?2",
            params![account_id, logical_request_id],
            read_reservation,
        )
        .optional()
        .map_err(Into::into)
}

fn reservation_for_request_tx(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
    logical_request_id: &str,
) -> Result<Option<RiskReservation>, RiskStoreError> {
    reservation_for_request_connection(transaction, account_id, logical_request_id)
}

fn reservation_by_id(
    connection: &Connection,
    reservation_id: &str,
) -> Result<Option<RiskReservation>, RiskStoreError> {
    connection
        .query_row(
            "SELECT reservation_id, account_id, instrument_id, strategy_id, source,
                    logical_request_id, reserved_delta_lots, remaining_delta_lots,
                    reserved_notional_nanos, state, created_at_unix_ms, updated_at_unix_ms,
                    expires_at_unix_ms
             FROM risk_reservations WHERE reservation_id = ?1",
            [reservation_id],
            read_reservation,
        )
        .optional()
        .map_err(Into::into)
}

fn read_reservation(row: &rusqlite::Row<'_>) -> rusqlite::Result<RiskReservation> {
    let source = parse_source(row.get::<_, String>(4)?).map_err(text_conversion_error)?;
    let state = parse_state(row.get::<_, String>(9)?).map_err(text_conversion_error)?;
    let notional = row
        .get::<_, String>(8)?
        .parse::<i128>()
        .map_err(conversion_error)?;
    Ok(RiskReservation {
        reservation_id: row.get(0)?,
        account_id: row.get(1)?,
        instrument_id: row.get(2)?,
        strategy_id: row.get(3)?,
        source,
        logical_request_id: row.get(5)?,
        reserved_delta_lots: row.get(6)?,
        remaining_delta_lots: row.get(7)?,
        reserved_notional_nanos: notional,
        state,
        created_at_unix_ms: row.get(10)?,
        updated_at_unix_ms: row.get(11)?,
        expires_at_unix_ms: row.get(12)?,
    })
}

fn state_name(state: ReservationState) -> &'static str {
    match state {
        ReservationState::Active => "ACTIVE",
        ReservationState::PartiallyConsumed => "PARTIALLY_CONSUMED",
        ReservationState::Consumed => "CONSUMED",
        ReservationState::Released => "RELEASED",
        ReservationState::UnknownHeld => "UNKNOWN_HELD",
        ReservationState::Orphaned => "ORPHANED",
    }
}

fn parse_state(value: String) -> Result<ReservationState, &'static str> {
    match value.as_str() {
        "ACTIVE" => Ok(ReservationState::Active),
        "PARTIALLY_CONSUMED" => Ok(ReservationState::PartiallyConsumed),
        "CONSUMED" => Ok(ReservationState::Consumed),
        "RELEASED" => Ok(ReservationState::Released),
        "UNKNOWN_HELD" => Ok(ReservationState::UnknownHeld),
        "ORPHANED" => Ok(ReservationState::Orphaned),
        _ => Err("unknown reservation state"),
    }
}

fn source_name(source: RiskSource) -> &'static str {
    match source {
        RiskSource::Manual => "MANUAL",
        RiskSource::Strategy => "STRATEGY",
        RiskSource::Ml => "ML",
        RiskSource::Ai => "AI",
        RiskSource::EmergencyOperator => "EMERGENCY_OPERATOR",
    }
}

fn parse_source(value: String) -> Result<RiskSource, &'static str> {
    match value.as_str() {
        "MANUAL" => Ok(RiskSource::Manual),
        "STRATEGY" => Ok(RiskSource::Strategy),
        "ML" => Ok(RiskSource::Ml),
        "AI" => Ok(RiskSource::Ai),
        "EMERGENCY_OPERATOR" => Ok(RiskSource::EmergencyOperator),
        _ => Err("unknown risk source"),
    }
}

fn text_conversion_error(message: &'static str) -> rusqlite::Error {
    conversion_error(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message,
    ))
}

fn conversion_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS risk_decisions (
    decision_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    policy_revision INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_risk_decisions_request
    ON risk_decisions(account_id, request_id);

CREATE TABLE IF NOT EXISTS risk_reservations (
    reservation_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    instrument_id TEXT NOT NULL,
    strategy_id TEXT,
    source TEXT NOT NULL,
    logical_request_id TEXT NOT NULL,
    reserved_delta_lots INTEGER NOT NULL,
    remaining_delta_lots INTEGER NOT NULL,
    reserved_notional_nanos TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL,
    expires_at_unix_ms INTEGER,
    UNIQUE(account_id, logical_request_id)
);
CREATE INDEX IF NOT EXISTS idx_risk_reservations_active
    ON risk_reservations(account_id, instrument_id, state);

CREATE TABLE IF NOT EXISTS risk_policies (
    account_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL,
    payload TEXT NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS risk_policy_audit (
    event_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    old_revision INTEGER,
    new_revision INTEGER NOT NULL,
    reason TEXT NOT NULL,
    payload TEXT NOT NULL,
    observed_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_risk_policy_audit_account
    ON risk_policy_audit(account_id, observed_at_unix_ms);
";

#[derive(Debug, Error)]
pub enum RiskStoreError {
    #[error("risk sqlite persistence failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("risk serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("risk reservation capacity exceeded")]
    CapacityExceeded,
    #[error("risk reservation capacity is invalid")]
    InvalidCapacity,
    #[error("risk reservation not found")]
    ReservationNotFound,
    #[error("invalid risk reservation transition")]
    InvalidTransition,
    #[error("risk arithmetic overflow")]
    ArithmeticOverflow,
    #[error("risk approval invariant violated: {0}")]
    ApprovalInvariantViolation(&'static str),
    #[error("stored risk numeric value is invalid: {0}")]
    StoredNumeric(String),
    #[error("risk policy revision changed")]
    PolicyRevisionConflict,
    #[error("risk policy mutation is invalid")]
    InvalidPolicyMutation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        RiskActionKind, RiskDecision, RiskOutcome, RiskReservation, RiskSource, RiskValidityContext,
    };

    fn decision(request: &str, reservation_id: &str, delta: i64) -> RiskDecision {
        RiskDecision {
            decision_id: RiskDecision::new_id(),
            request_id: request.to_owned(),
            policy_revision: 1,
            account_id: "account-1".to_owned(),
            action: RiskActionKind::DirectionalOrder,
            requested_delta_lots: delta,
            approved_delta_lots: delta,
            outcome: RiskOutcome::Approve,
            reasons: Vec::new(),
            reservation_id: Some(reservation_id.to_owned()),
            expires_at_unix_ms: None,
            validity: RiskValidityContext {
                runtime_epoch: 1,
                reconciliation_revision: 1,
                position_revision: 1,
                order_revision: 1,
                market_data_as_of_unix_ms: Some(1),
                instrument_constraints_revision: 1,
                policy_revision: 1,
                execution_authorization_revision: 1,
            },
        }
    }

    fn reservation(request: &str, delta: i64) -> RiskReservation {
        RiskReservation {
            reservation_id: RiskReservation::new_id(),
            account_id: "account-1".to_owned(),
            instrument_id: "instrument-1".to_owned(),
            strategy_id: None,
            source: RiskSource::Manual,
            logical_request_id: request.to_owned(),
            reserved_delta_lots: delta,
            remaining_delta_lots: delta,
            reserved_notional_nanos: i128::from(delta) * 1_000_000_000,
            state: ReservationState::Active,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            expires_at_unix_ms: None,
        }
    }

    #[test]
    fn reservation_is_idempotent_by_logical_request() -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        let first = store.reserve_atomic(&reservation("req-1", 5), 10)?;
        let replay = store.reserve_atomic(&reservation("req-1", 5), 10)?;
        assert_eq!(first.reservation_id, replay.reservation_id);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn reservations_cannot_oversubscribe_capacity() -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        store.reserve_atomic(&reservation("req-1", 6), 10)?;
        let result = store.reserve_atomic(&reservation("req-2", 5), 10);
        assert!(matches!(result, Err(RiskStoreError::CapacityExceeded)));
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn unknown_state_keeps_capacity_reserved() -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        let first = store.reserve_atomic(&reservation("req-1", 6), 10)?;
        store.update_reservation(
            &first.reservation_id,
            &[ReservationState::Active],
            6,
            ReservationState::UnknownHeld,
            2,
        )?;
        let result = store.reserve_atomic(&reservation("req-2", 5), 10);
        assert!(matches!(result, Err(RiskStoreError::CapacityExceeded)));
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn approval_and_reservation_are_persisted_in_one_transaction() -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        let reservation = reservation("req-atomic", 4);
        let decision = decision("req-atomic", &reservation.reservation_id, 4);

        let persisted = store.persist_approval_atomic(
            &decision,
            &reservation,
            ReservationCapacity {
                max_account_reserved_notional_nanos: Some(10_000_000_000),
                max_instrument_reserved_abs_lots: Some(10),
            },
        )?;

        assert_eq!(persisted.decision.decision_id, decision.decision_id);
        assert_eq!(
            persisted.reservation.reservation_id,
            reservation.reservation_id
        );
        assert_eq!(
            store
                .decision(&decision.decision_id)?
                .expect("decision")
                .reservation_id,
            Some(reservation.reservation_id.clone())
        );
        assert_eq!(
            store
                .reservation_for_request("account-1", "req-atomic")?
                .expect("reservation")
                .reservation_id,
            reservation.reservation_id
        );
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn failed_capacity_check_persists_neither_decision_nor_reservation()
    -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        let reservation = reservation("req-too-large", 6);
        let decision = decision("req-too-large", &reservation.reservation_id, 6);

        let result = store.persist_approval_atomic(
            &decision,
            &reservation,
            ReservationCapacity {
                max_account_reserved_notional_nanos: Some(5_000_000_000),
                max_instrument_reserved_abs_lots: Some(5),
            },
        );
        assert!(matches!(result, Err(RiskStoreError::CapacityExceeded)));
        assert!(store.decision(&decision.decision_id)?.is_none());
        assert!(
            store
                .reservation_for_request("account-1", "req-too-large")?
                .is_none()
        );
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn replay_returns_the_original_persisted_approval() -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        let original_reservation = reservation("req-replay", 3);
        let original_decision = decision("req-replay", &original_reservation.reservation_id, 3);
        let capacity = ReservationCapacity {
            max_account_reserved_notional_nanos: Some(10_000_000_000),
            max_instrument_reserved_abs_lots: Some(10),
        };
        let first =
            store.persist_approval_atomic(&original_decision, &original_reservation, capacity)?;

        let replay_reservation = reservation("req-replay", 3);
        let replay_decision = decision("req-replay", &replay_reservation.reservation_id, 3);
        let replay =
            store.persist_approval_atomic(&replay_decision, &replay_reservation, capacity)?;

        assert_eq!(first, replay);
        assert_eq!(
            store.approval_for_request("account-1", "req-replay")?,
            Some(first)
        );
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn policy_update_requires_exact_revision_and_round_trips() -> Result<(), RiskStoreError> {
        let path = std::env::temp_dir().join(format!("vox-risk-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteRiskStore::open(&path)?;
        let mut first = RiskPolicySet::fail_closed(1);
        first.state = crate::model::RiskState::Halted;
        assert_eq!(
            store.put_policy("account-1", None, &first, "initial halt", 1)?,
            first
        );

        let mut second = first.clone();
        second.revision = 2;
        second.state = crate::model::RiskState::ReduceOnly;
        assert!(matches!(
            store.put_policy("account-1", Some(9), &second, "stale write", 2),
            Err(RiskStoreError::PolicyRevisionConflict)
        ));
        store.put_policy("account-1", Some(1), &second, "operator recovery", 2)?;
        assert_eq!(store.policy("account-1")?, Some(second));
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
