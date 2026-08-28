use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model::{
    BrokerEvent, BrokerIdentityLinks, DerivedPositionExpectation, JournalState, MutationRecord,
    ReasonCode, ReconciliationCheckpoint, ReconciliationDisposition, RuntimeAuditRecord,
    RuntimeScope, RuntimeState, StateTransition, StoreCounts,
};
use crate::ports::{RuntimeStore, StoreError};

const SCHEMA_VERSION: u32 = 3;
const SOFTWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS runtime_instance (
    scope_key TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    environment TEXT NOT NULL,
    broker_account_id TEXT NOT NULL,
    connection_ref TEXT NOT NULL,
    credential_ref TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch >= 1),
    owner_id TEXT NOT NULL,
    lease_active INTEGER NOT NULL CHECK(lease_active IN (0, 1)),
    started_at_unix_ms INTEGER NOT NULL,
    clean_shutdown_at_unix_ms INTEGER,
    software_version TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    last_state TEXT NOT NULL,
    last_reason TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS mutation_journal (
    logical_request_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    mutation_kind TEXT NOT NULL,
    state TEXT NOT NULL,
    redacted_request_evidence TEXT NOT NULL CHECK(length(redacted_request_evidence) BETWEEN 1 AND 2048),
    broker_evidence_ref TEXT,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL,
    correlation_id TEXT NOT NULL,
    reconciliation_disposition TEXT,
    runtime_epoch INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS mutation_scope_state_idx
    ON mutation_journal(scope_key, state);

CREATE TABLE IF NOT EXISTS broker_identity_links (
    logical_request_id TEXT PRIMARY KEY REFERENCES mutation_journal(logical_request_id) ON DELETE RESTRICT,
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    broker_order_id TEXT,
    replacement_broker_order_id TEXT,
    broker_stop_order_id TEXT,
    updated_at_unix_ms INTEGER NOT NULL,
    runtime_epoch INTEGER NOT NULL,
    UNIQUE(scope_key, broker_order_id),
    UNIQUE(scope_key, replacement_broker_order_id),
    UNIQUE(scope_key, broker_stop_order_id)
);

CREATE TABLE IF NOT EXISTS provider_operation_links (
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    logical_request_id TEXT NOT NULL REFERENCES mutation_journal(logical_request_id) ON DELETE RESTRICT,
    provider_operation_id TEXT NOT NULL,
    PRIMARY KEY(scope_key, logical_request_id, provider_operation_id)
);

CREATE TABLE IF NOT EXISTS broker_fill_links (
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    logical_request_id TEXT NOT NULL REFERENCES mutation_journal(logical_request_id) ON DELETE RESTRICT,
    broker_fill_id TEXT NOT NULL,
    PRIMARY KEY(scope_key, broker_fill_id)
);

CREATE TABLE IF NOT EXISTS reconciliation_checkpoint (
    scope_key TEXT PRIMARY KEY REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    reconciliation_id TEXT NOT NULL,
    operations_cursor TEXT,
    snapshot_observed_at_unix_ms INTEGER NOT NULL,
    completed_at_unix_ms INTEGER NOT NULL,
    runtime_epoch INTEGER NOT NULL,
    accounts_complete INTEGER NOT NULL CHECK(accounts_complete IN (0, 1)),
    portfolio_complete INTEGER NOT NULL CHECK(portfolio_complete IN (0, 1)),
    positions_complete INTEGER NOT NULL CHECK(positions_complete IN (0, 1)),
    orders_complete INTEGER NOT NULL CHECK(orders_complete IN (0, 1)),
    stops_complete INTEGER NOT NULL CHECK(stops_complete IN (0, 1)),
    operations_complete INTEGER NOT NULL CHECK(operations_complete IN (0, 1))
);

CREATE TABLE IF NOT EXISTS processed_broker_event (
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    event_class TEXT NOT NULL,
    stable_event_id TEXT NOT NULL,
    first_seen_at_unix_ms INTEGER NOT NULL,
    runtime_epoch INTEGER NOT NULL,
    PRIMARY KEY(scope_key, event_class, stable_event_id)
);
CREATE INDEX IF NOT EXISTS processed_event_retention_idx
    ON processed_broker_event(scope_key, first_seen_at_unix_ms);

CREATE TABLE IF NOT EXISTS derived_position_expectation (
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    instrument_uid TEXT NOT NULL,
    expected_quantity_units INTEGER NOT NULL,
    based_on_logical_request_id TEXT NOT NULL REFERENCES mutation_journal(logical_request_id) ON DELETE RESTRICT,
    runtime_epoch INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL,
    PRIMARY KEY(scope_key, instrument_uid)
);

CREATE TABLE IF NOT EXISTS runtime_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    runtime_epoch INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    correlation_id TEXT NOT NULL,
    redacted_detail TEXT NOT NULL CHECK(length(redacted_detail) <= 2048),
    observed_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS runtime_audit_retention_idx
    ON runtime_audit(scope_key, observed_at_unix_ms);
"#;

const MIGRATION_V2: &str = r#"
CREATE TRIGGER IF NOT EXISTS mutation_journal_capacity
BEFORE INSERT ON mutation_journal
WHEN (SELECT count(*) FROM mutation_journal) >= 1000000
BEGIN
    SELECT RAISE(ABORT, 'mutation journal capacity reached');
END;

CREATE TRIGGER IF NOT EXISTS processed_broker_event_capacity
AFTER INSERT ON processed_broker_event
BEGIN
    DELETE FROM processed_broker_event
    WHERE rowid IN (
        SELECT rowid FROM processed_broker_event
        WHERE scope_key=NEW.scope_key
        ORDER BY first_seen_at_unix_ms DESC, rowid DESC
        LIMIT -1 OFFSET 100000
    );
END;

CREATE TRIGGER IF NOT EXISTS runtime_audit_capacity
AFTER INSERT ON runtime_audit
BEGIN
    DELETE FROM runtime_audit
    WHERE audit_id IN (
        SELECT audit_id FROM runtime_audit
        WHERE scope_key=NEW.scope_key
        ORDER BY observed_at_unix_ms DESC, audit_id DESC
        LIMIT -1 OFFSET 20000
    );
END;
"#;

const MIGRATION_V3: &str = r#"
UPDATE mutation_journal
SET redacted_request_evidence =
    '{"command_kind":' || mutation_kind ||
    ',"instrument_ref":null,"quantity_lots":null,"price_present":false,"protection_kind":null}';
CREATE TABLE broker_identity_links_v3 (
    logical_request_id TEXT PRIMARY KEY REFERENCES mutation_journal(logical_request_id) ON DELETE RESTRICT,
    scope_key TEXT NOT NULL REFERENCES runtime_instance(scope_key) ON DELETE RESTRICT,
    broker_order_id TEXT,
    replacement_broker_order_id TEXT,
    broker_stop_order_id TEXT,
    updated_at_unix_ms INTEGER NOT NULL,
    runtime_epoch INTEGER NOT NULL
);
INSERT INTO broker_identity_links_v3
SELECT logical_request_id, scope_key, broker_order_id, replacement_broker_order_id,
       broker_stop_order_id, updated_at_unix_ms, runtime_epoch
FROM broker_identity_links;
DROP TABLE broker_identity_links;
ALTER TABLE broker_identity_links_v3 RENAME TO broker_identity_links;
CREATE INDEX broker_identity_order_idx
    ON broker_identity_links(scope_key, broker_order_id);
CREATE INDEX broker_identity_replacement_idx
    ON broker_identity_links(scope_key, replacement_broker_order_id);
CREATE INDEX broker_identity_stop_idx
    ON broker_identity_links(scope_key, broker_stop_order_id);
"#;

struct StoreInner {
    connection: Mutex<Connection>,
    lock_file: File,
    path: PathBuf,
}

impl Drop for StoreInner {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock_file);
    }
}

#[derive(Clone)]
pub struct SqliteRuntimeStore {
    inner: Arc<StoreInner>,
}

impl SqliteRuntimeStore {
    /// Opens and migrates SQLite without blocking a Tokio core worker.
    pub async fn open_async(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || Self::open(path))
            .await
            .map_err(|error| StoreError::BlockingTask(error.to_string()))?
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(persistence)?;
        }
        let lock_path = path.with_extension("runtime.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(persistence)?;
        lock_file
            .try_lock_exclusive()
            .map_err(|_| StoreError::OwnershipUnavailable)?;

        let mut connection = Connection::open(&path).map_err(persistence)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(persistence)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(persistence)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(persistence)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(persistence)?;
        migrate(&mut connection)?;

        Ok(Self {
            inner: Arc::new(StoreInner {
                connection: Mutex::new(connection),
                lock_file,
                path,
            }),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    pub fn configuration(&self) -> Result<SqliteConfiguration, StoreError> {
        let connection = self.connection()?;
        Ok(SqliteConfiguration {
            journal_mode: connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .map_err(persistence)?,
            foreign_keys: connection
                .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
                .map_err(persistence)?
                == 1,
            synchronous: connection
                .pragma_query_value(None, "synchronous", |row| row.get(0))
                .map_err(persistence)?,
            user_version: connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(persistence)?,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.inner
            .connection
            .lock()
            .map_err(|error| StoreError::Persistence(error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteConfiguration {
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub synchronous: i64,
    pub user_version: u32,
}

impl RuntimeStore for SqliteRuntimeStore {
    fn acquire_ownership(
        &self,
        scope: &RuntimeScope,
        owner_id: &str,
        started_at_unix_ms: i64,
    ) -> Result<u64, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        let existing_epoch = transaction
            .query_row(
                "SELECT epoch FROM runtime_instance WHERE scope_key = ?1",
                [scope.key()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(persistence)?
            .unwrap_or(0);
        let epoch = existing_epoch
            .checked_add(1)
            .ok_or_else(|| StoreError::Corrupt("runtime epoch overflow".into()))?;
        let changed = transaction
            .execute(
                "INSERT INTO runtime_instance (
                    scope_key, provider, environment, broker_account_id, connection_ref,
                    credential_ref, epoch, owner_id, lease_active, started_at_unix_ms,
                    clean_shutdown_at_unix_ms, software_version, schema_version,
                    last_state, last_reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, NULL, ?10, ?11, ?12, ?13)
                 ON CONFLICT(scope_key) DO UPDATE SET
                    provider=excluded.provider,
                    environment=excluded.environment,
                    broker_account_id=excluded.broker_account_id,
                    connection_ref=excluded.connection_ref,
                    credential_ref=excluded.credential_ref,
                    epoch=excluded.epoch,
                    owner_id=excluded.owner_id,
                    lease_active=1,
                    started_at_unix_ms=excluded.started_at_unix_ms,
                    clean_shutdown_at_unix_ms=NULL,
                    software_version=excluded.software_version,
                    schema_version=excluded.schema_version,
                    last_state=excluded.last_state,
                    last_reason=excluded.last_reason",
                params![
                    scope.key(),
                    encode(&scope.provider)?,
                    encode(&scope.environment)?,
                    scope.broker_account_id,
                    scope.connection_ref.as_str(),
                    scope.credential_ref.as_str(),
                    epoch,
                    owner_id,
                    started_at_unix_ms,
                    SOFTWARE_VERSION,
                    SCHEMA_VERSION,
                    encode(&RuntimeState::Starting)?,
                    encode(&ReasonCode::Startup)?,
                ],
            )
            .map_err(persistence)?;
        changed_one(changed)?;
        insert_audit(
            &transaction,
            &RuntimeAuditRecord {
                scope_key: scope.key(),
                runtime_epoch: to_u64(epoch)?,
                event_type: "OWNERSHIP_ACQUIRED".into(),
                reason_code: ReasonCode::Startup,
                correlation_id: owner_id.to_owned(),
                redacted_detail: "single-writer lease acquired and epoch incremented".into(),
                observed_at_unix_ms: started_at_unix_ms,
            },
        )?;
        transaction.commit().map_err(persistence)?;
        to_u64(epoch)
    }

    fn verify_epoch(&self, scope_key: &str, runtime_epoch: u64) -> Result<(), StoreError> {
        let connection = self.connection()?;
        verify_epoch_connection(&connection, scope_key, runtime_epoch)
    }

    fn release_ownership(
        &self,
        scope_key: &str,
        runtime_epoch: u64,
        clean_shutdown_at_unix_ms: i64,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        let changed = transaction
            .execute(
                "UPDATE runtime_instance
                 SET lease_active=0, clean_shutdown_at_unix_ms=?3, last_state=?4, last_reason=?5
                 WHERE scope_key=?1 AND epoch=?2 AND lease_active=1",
                params![
                    scope_key,
                    to_i64(runtime_epoch)?,
                    clean_shutdown_at_unix_ms,
                    encode(&RuntimeState::Stopped)?,
                    encode(&ReasonCode::ShutdownComplete)?,
                ],
            )
            .map_err(persistence)?;
        changed_one(changed)?;
        insert_audit(
            &transaction,
            &RuntimeAuditRecord {
                scope_key: scope_key.to_owned(),
                runtime_epoch,
                event_type: "OWNERSHIP_RELEASED".into(),
                reason_code: ReasonCode::ShutdownComplete,
                correlation_id: format!("shutdown-epoch-{runtime_epoch}"),
                redacted_detail: "clean shutdown marker committed before ownership release".into(),
                observed_at_unix_ms: clean_shutdown_at_unix_ms,
            },
        )?;
        transaction.commit().map_err(persistence)
    }

    fn record_transition(&self, transition: &StateTransition) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        let scope_key = transition.scope_key.clone();
        verify_epoch_transaction(&transaction, &scope_key, transition.runtime_epoch)?;
        let changed = transaction
            .execute(
                "UPDATE runtime_instance SET last_state=?3, last_reason=?4
                 WHERE scope_key=?1 AND epoch=?2 AND lease_active=1",
                params![
                    scope_key,
                    to_i64(transition.runtime_epoch)?,
                    encode(&transition.to)?,
                    encode(&transition.reason_code)?,
                ],
            )
            .map_err(persistence)?;
        changed_one(changed)?;
        insert_audit(
            &transaction,
            &RuntimeAuditRecord {
                scope_key,
                runtime_epoch: transition.runtime_epoch,
                event_type: "STATE_TRANSITION".into(),
                reason_code: transition.reason_code,
                correlation_id: transition.correlation_id.clone(),
                redacted_detail: format!(
                    "{:?}->{:?}: {}",
                    transition.from, transition.to, transition.detail
                ),
                observed_at_unix_ms: transition.observed_at_unix_ms,
            },
        )?;
        transaction.commit().map_err(persistence)
    }

    fn insert_mutation(&self, record: &MutationRecord) -> Result<(), StoreError> {
        let connection = self.connection()?;
        verify_epoch_connection(&connection, &record.scope_key, record.runtime_epoch)?;
        let request_evidence = encode(&record.request_evidence)?;
        let disposition = record
            .reconciliation_disposition
            .as_ref()
            .map(encode)
            .transpose()?;
        connection
            .execute(
                "INSERT INTO mutation_journal (
                    logical_request_id, scope_key, mutation_kind, state,
                    redacted_request_evidence, broker_evidence_ref, created_at_unix_ms,
                    updated_at_unix_ms, correlation_id, reconciliation_disposition,
                    runtime_epoch
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    record.logical_request_id,
                    record.scope_key,
                    encode(&record.kind)?,
                    encode(&record.state)?,
                    request_evidence,
                    record.broker_evidence_ref,
                    record.created_at_unix_ms,
                    record.updated_at_unix_ms,
                    record.correlation_id,
                    disposition,
                    to_i64(record.runtime_epoch)?,
                ],
            )
            .map(|_| ())
            .map_err(|error| {
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                    StoreError::DuplicateMutation
                } else {
                    persistence(error)
                }
            })
    }

    fn claim_dispatch_unknown(
        &self,
        scope_key: &str,
        logical_request_id: &str,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<MutationRecord, StoreError> {
        self.transition_mutation(
            scope_key,
            logical_request_id,
            &[JournalState::NotDispatched],
            JournalState::UnknownAfterDispatch,
            None,
            None,
            runtime_epoch,
            now_unix_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn transition_mutation(
        &self,
        scope_key: &str,
        logical_request_id: &str,
        expected: &[JournalState],
        target: JournalState,
        broker_evidence_ref: Option<&str>,
        disposition: Option<&ReconciliationDisposition>,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<MutationRecord, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        verify_epoch_transaction(&transaction, scope_key, runtime_epoch)?;
        let mut record = load_mutation(&transaction, logical_request_id)?
            .ok_or(StoreError::InvalidMutationTransition)?;
        if record.scope_key != scope_key || !expected.contains(&record.state) {
            return Err(StoreError::InvalidMutationTransition);
        }
        record.state = target;
        record.broker_evidence_ref = broker_evidence_ref.map(str::to_owned);
        record.reconciliation_disposition = disposition.cloned();
        record.updated_at_unix_ms = now_unix_ms;
        record.runtime_epoch = runtime_epoch;
        record = record
            .validated()
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        let encoded_disposition = disposition.map(encode).transpose()?;
        let changed = transaction
            .execute(
                "UPDATE mutation_journal SET state=?2, broker_evidence_ref=?3,
                    reconciliation_disposition=?4, updated_at_unix_ms=?5, runtime_epoch=?6
                 WHERE logical_request_id=?1",
                params![
                    logical_request_id,
                    encode(&target)?,
                    broker_evidence_ref,
                    encoded_disposition,
                    now_unix_ms,
                    to_i64(runtime_epoch)?,
                ],
            )
            .map_err(persistence)?;
        changed_one(changed)?;
        let reason_code = if target == JournalState::UnknownAfterDispatch {
            ReasonCode::UnknownMutation
        } else {
            ReasonCode::ReconciliationComplete
        };
        insert_audit(
            &transaction,
            &RuntimeAuditRecord {
                scope_key: scope_key.to_owned(),
                runtime_epoch,
                event_type: format!("MUTATION_{target:?}"),
                reason_code,
                correlation_id: record.correlation_id.clone(),
                redacted_detail: format!(
                    "logical_request_id={logical_request_id}; disposition={}",
                    disposition.map_or_else(|| "none".into(), |value| format!("{value:?}"))
                ),
                observed_at_unix_ms: now_unix_ms,
            },
        )?;
        transaction.commit().map_err(persistence)?;
        Ok(record)
    }

    fn unresolved_mutations(&self, scope_key: &str) -> Result<Vec<MutationRecord>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT logical_request_id, scope_key, mutation_kind, state,
                    redacted_request_evidence, broker_evidence_ref, created_at_unix_ms,
                    updated_at_unix_ms, correlation_id, reconciliation_disposition,
                    runtime_epoch
                 FROM mutation_journal
                 WHERE scope_key=?1 AND state IN (?2, ?3)
                 ORDER BY created_at_unix_ms, logical_request_id",
            )
            .map_err(persistence)?;
        let rows = statement
            .query_map(
                params![
                    scope_key,
                    encode(&JournalState::Dispatching)?,
                    encode(&JournalState::UnknownAfterDispatch)?,
                ],
                mutation_from_row,
            )
            .map_err(persistence)?;
        rows.map(|row| row.map_err(persistence)).collect()
    }

    fn all_identity_links(&self, scope_key: &str) -> Result<Vec<BrokerIdentityLinks>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT logical_request_id, broker_order_id, replacement_broker_order_id,
                    broker_stop_order_id
                 FROM broker_identity_links WHERE scope_key=?1 ORDER BY logical_request_id",
            )
            .map_err(persistence)?;
        let base = statement
            .query_map([scope_key], |row| {
                Ok(BrokerIdentityLinks {
                    logical_request_id: row.get(0)?,
                    broker_order_id: row.get(1)?,
                    replacement_broker_order_id: row.get(2)?,
                    broker_stop_order_id: row.get(3)?,
                    provider_operation_ids: Default::default(),
                    broker_fill_ids: Default::default(),
                })
            })
            .map_err(persistence)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(persistence)?;
        base.into_iter()
            .map(|mut links| {
                links.provider_operation_ids = string_set(
                    &connection,
                    "SELECT provider_operation_id FROM provider_operation_links
                     WHERE scope_key=?1 AND logical_request_id=?2",
                    scope_key,
                    &links.logical_request_id,
                )?;
                links.broker_fill_ids = string_set(
                    &connection,
                    "SELECT broker_fill_id FROM broker_fill_links
                     WHERE scope_key=?1 AND logical_request_id=?2",
                    scope_key,
                    &links.logical_request_id,
                )?;
                Ok(links)
            })
            .collect()
    }

    fn upsert_identity_links(
        &self,
        scope_key: &str,
        links: &BrokerIdentityLinks,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        verify_epoch_transaction(&transaction, scope_key, runtime_epoch)?;
        upsert_links(&transaction, scope_key, links, runtime_epoch, now_unix_ms)?;
        transaction.commit().map_err(persistence)
    }

    fn load_checkpoint(
        &self,
        scope_key: &str,
    ) -> Result<Option<ReconciliationCheckpoint>, StoreError> {
        self.connection()?
            .query_row(
                "SELECT scope_key, reconciliation_id, operations_cursor,
                    snapshot_observed_at_unix_ms, completed_at_unix_ms, runtime_epoch,
                    accounts_complete, portfolio_complete, positions_complete,
                    orders_complete, stops_complete, operations_complete
                 FROM reconciliation_checkpoint WHERE scope_key=?1",
                [scope_key],
                |row| {
                    Ok(ReconciliationCheckpoint {
                        scope_key: row.get(0)?,
                        reconciliation_id: row.get(1)?,
                        operations_cursor: row.get(2)?,
                        snapshot_observed_at_unix_ms: row.get(3)?,
                        completed_at_unix_ms: row.get(4)?,
                        runtime_epoch: to_u64_sql(row.get(5)?)?,
                        accounts_complete: row.get(6)?,
                        portfolio_complete: row.get(7)?,
                        positions_complete: row.get(8)?,
                        orders_complete: row.get(9)?,
                        stops_complete: row.get(10)?,
                        operations_complete: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(persistence)
    }

    fn discard_checkpoint(&self, scope_key: &str, runtime_epoch: u64) -> Result<(), StoreError> {
        let connection = self.connection()?;
        verify_epoch_connection(&connection, scope_key, runtime_epoch)?;
        connection
            .execute(
                "DELETE FROM reconciliation_checkpoint WHERE scope_key=?1",
                [scope_key],
            )
            .map(|_| ())
            .map_err(persistence)
    }

    fn commit_reconciliation(
        &self,
        checkpoint: &ReconciliationCheckpoint,
        resolved: &[MutationRecord],
        links: &[BrokerIdentityLinks],
        readiness_state: RuntimeState,
        reason_code: ReasonCode,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        verify_epoch_transaction(
            &transaction,
            &checkpoint.scope_key,
            checkpoint.runtime_epoch,
        )?;
        for record in resolved {
            if record.scope_key != checkpoint.scope_key
                || !matches!(record.state, JournalState::Reconciled)
            {
                return Err(StoreError::InvalidMutationTransition);
            }
            let disposition = record
                .reconciliation_disposition
                .as_ref()
                .map(encode)
                .transpose()?;
            let changed = transaction
                .execute(
                    "UPDATE mutation_journal SET state=?2, broker_evidence_ref=?3,
                        reconciliation_disposition=?4, updated_at_unix_ms=?5, runtime_epoch=?6
                     WHERE logical_request_id=?1 AND scope_key=?7
                       AND state IN (?8, ?9)",
                    params![
                        record.logical_request_id,
                        encode(&record.state)?,
                        record.broker_evidence_ref,
                        disposition,
                        record.updated_at_unix_ms,
                        to_i64(checkpoint.runtime_epoch)?,
                        checkpoint.scope_key,
                        encode(&JournalState::Dispatching)?,
                        encode(&JournalState::UnknownAfterDispatch)?,
                    ],
                )
                .map_err(persistence)?;
            if changed != 1 {
                return Err(StoreError::Corrupt(format!(
                    "reconciliation target {} is missing or no longer unresolved",
                    record.logical_request_id
                )));
            }
            insert_audit(
                &transaction,
                &RuntimeAuditRecord {
                    scope_key: checkpoint.scope_key.clone(),
                    runtime_epoch: checkpoint.runtime_epoch,
                    event_type: "UNKNOWN_RESOLVED".into(),
                    reason_code: ReasonCode::ReconciliationComplete,
                    correlation_id: record.correlation_id.clone(),
                    redacted_detail: format!(
                        "logical_request_id={}; disposition={}",
                        record.logical_request_id,
                        record.reconciliation_disposition.as_ref().map_or_else(
                            || "authoritative broker evidence".into(),
                            |value| format!("{value:?}"),
                        )
                    ),
                    observed_at_unix_ms: record.updated_at_unix_ms,
                },
            )?;
        }
        for link in links {
            upsert_links(
                &transaction,
                &checkpoint.scope_key,
                link,
                checkpoint.runtime_epoch,
                checkpoint.completed_at_unix_ms,
            )?;
        }
        let changed = transaction
            .execute(
                "INSERT INTO reconciliation_checkpoint (
                    scope_key, reconciliation_id, operations_cursor,
                    snapshot_observed_at_unix_ms, completed_at_unix_ms, runtime_epoch,
                    accounts_complete, portfolio_complete, positions_complete,
                    orders_complete, stops_complete, operations_complete
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(scope_key) DO UPDATE SET
                    reconciliation_id=excluded.reconciliation_id,
                    operations_cursor=excluded.operations_cursor,
                    snapshot_observed_at_unix_ms=excluded.snapshot_observed_at_unix_ms,
                    completed_at_unix_ms=excluded.completed_at_unix_ms,
                    runtime_epoch=excluded.runtime_epoch,
                    accounts_complete=excluded.accounts_complete,
                    portfolio_complete=excluded.portfolio_complete,
                    positions_complete=excluded.positions_complete,
                    orders_complete=excluded.orders_complete,
                    stops_complete=excluded.stops_complete,
                    operations_complete=excluded.operations_complete",
                params![
                    checkpoint.scope_key,
                    checkpoint.reconciliation_id,
                    checkpoint.operations_cursor,
                    checkpoint.snapshot_observed_at_unix_ms,
                    checkpoint.completed_at_unix_ms,
                    to_i64(checkpoint.runtime_epoch)?,
                    checkpoint.accounts_complete,
                    checkpoint.portfolio_complete,
                    checkpoint.positions_complete,
                    checkpoint.orders_complete,
                    checkpoint.stops_complete,
                    checkpoint.operations_complete,
                ],
            )
            .map_err(persistence)?;
        changed_one(changed)?;
        let runtime_changed = transaction
            .execute(
                "UPDATE runtime_instance SET last_state=?3, last_reason=?4
                 WHERE scope_key=?1 AND epoch=?2 AND lease_active=1",
                params![
                    checkpoint.scope_key,
                    to_i64(checkpoint.runtime_epoch)?,
                    encode(&readiness_state)?,
                    encode(&reason_code)?,
                ],
            )
            .map_err(persistence)?;
        changed_one(runtime_changed)?;
        insert_audit(
            &transaction,
            &RuntimeAuditRecord {
                scope_key: checkpoint.scope_key.clone(),
                runtime_epoch: checkpoint.runtime_epoch,
                event_type: "RECONCILIATION_COMMITTED".into(),
                reason_code,
                correlation_id: checkpoint.reconciliation_id.clone(),
                redacted_detail: format!(
                    "readiness={readiness_state:?}; complete={}",
                    checkpoint.complete()
                ),
                observed_at_unix_ms: checkpoint.completed_at_unix_ms,
            },
        )?;
        transaction.commit().map_err(persistence)
    }

    fn record_broker_event(
        &self,
        scope_key: &str,
        event: &BrokerEvent,
        first_seen_at_unix_ms: i64,
    ) -> Result<bool, StoreError> {
        let connection = self.connection()?;
        verify_epoch_connection(&connection, scope_key, event.runtime_epoch)?;
        let changed = connection
            .execute(
                "INSERT OR IGNORE INTO processed_broker_event (
                    scope_key, event_class, stable_event_id, first_seen_at_unix_ms, runtime_epoch
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    scope_key,
                    encode(&event.event_class)?,
                    event.stable_event_id,
                    first_seen_at_unix_ms,
                    to_i64(event.runtime_epoch)?,
                ],
            )
            .map_err(persistence)?;
        Ok(changed == 1)
    }

    fn expected_positions(
        &self,
        scope_key: &str,
    ) -> Result<Vec<DerivedPositionExpectation>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT scope_key, instrument_uid, expected_quantity_units,
                    based_on_logical_request_id, runtime_epoch, updated_at_unix_ms
                 FROM derived_position_expectation WHERE scope_key=?1
                 ORDER BY instrument_uid",
            )
            .map_err(persistence)?;
        statement
            .query_map([scope_key], |row| {
                Ok(DerivedPositionExpectation {
                    scope_key: row.get(0)?,
                    instrument_uid: row.get(1)?,
                    expected_quantity_units: row.get(2)?,
                    based_on_logical_request_id: row.get(3)?,
                    runtime_epoch: to_u64_sql(row.get(4)?)?,
                    updated_at_unix_ms: row.get(5)?,
                })
            })
            .map_err(persistence)?
            .map(|row| row.map_err(persistence))
            .collect()
    }

    fn set_expected_position(
        &self,
        expectation: &DerivedPositionExpectation,
    ) -> Result<(), StoreError> {
        let connection = self.connection()?;
        verify_epoch_connection(
            &connection,
            &expectation.scope_key,
            expectation.runtime_epoch,
        )?;
        connection
            .execute(
                "INSERT INTO derived_position_expectation (
                    scope_key, instrument_uid, expected_quantity_units,
                    based_on_logical_request_id, runtime_epoch, updated_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(scope_key, instrument_uid) DO UPDATE SET
                    expected_quantity_units=excluded.expected_quantity_units,
                    based_on_logical_request_id=excluded.based_on_logical_request_id,
                    runtime_epoch=excluded.runtime_epoch,
                    updated_at_unix_ms=excluded.updated_at_unix_ms",
                params![
                    expectation.scope_key,
                    expectation.instrument_uid,
                    expectation.expected_quantity_units,
                    expectation.based_on_logical_request_id,
                    to_i64(expectation.runtime_epoch)?,
                    expectation.updated_at_unix_ms,
                ],
            )
            .map(|_| ())
            .map_err(persistence)
    }

    fn append_audit(&self, record: &RuntimeAuditRecord) -> Result<(), StoreError> {
        let connection = self.connection()?;
        verify_epoch_connection(&connection, &record.scope_key, record.runtime_epoch)?;
        insert_audit(&connection, record)
    }

    fn compact(
        &self,
        scope_key: &str,
        retain_after_unix_ms: i64,
        max_events: u32,
        max_audit: u32,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(persistence)?;
        transaction
            .execute(
                "DELETE FROM processed_broker_event
                 WHERE scope_key=?1 AND first_seen_at_unix_ms < ?2",
                params![scope_key, retain_after_unix_ms],
            )
            .map_err(persistence)?;
        transaction
            .execute(
                "DELETE FROM processed_broker_event WHERE rowid IN (
                    SELECT rowid FROM processed_broker_event WHERE scope_key=?1
                    ORDER BY first_seen_at_unix_ms DESC, rowid DESC LIMIT -1 OFFSET ?2
                 )",
                params![scope_key, max_events],
            )
            .map_err(persistence)?;
        transaction
            .execute(
                "DELETE FROM runtime_audit
                 WHERE scope_key=?1 AND observed_at_unix_ms < ?2",
                params![scope_key, retain_after_unix_ms],
            )
            .map_err(persistence)?;
        transaction
            .execute(
                "DELETE FROM runtime_audit WHERE audit_id IN (
                    SELECT audit_id FROM runtime_audit WHERE scope_key=?1
                    ORDER BY observed_at_unix_ms DESC, audit_id DESC LIMIT -1 OFFSET ?2
                 )",
                params![scope_key, max_audit],
            )
            .map_err(persistence)?;
        transaction.commit().map_err(persistence)
    }

    fn counts(&self, scope_key: &str) -> Result<StoreCounts, StoreError> {
        let connection = self.connection()?;
        let unresolved_unknown_count = connection
            .query_row(
                "SELECT count(*) FROM mutation_journal
                 WHERE scope_key=?1 AND state IN (?2, ?3)",
                params![
                    scope_key,
                    encode(&JournalState::Dispatching)?,
                    encode(&JournalState::UnknownAfterDispatch)?,
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(persistence)?;
        let linked_open_order_count = connection
            .query_row(
                "SELECT count(*) FROM broker_identity_links WHERE scope_key=?1
                 AND (broker_order_id IS NOT NULL OR replacement_broker_order_id IS NOT NULL)",
                [scope_key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(persistence)?;
        let linked_active_stop_count = connection
            .query_row(
                "SELECT count(*) FROM broker_identity_links WHERE scope_key=?1
                 AND broker_stop_order_id IS NOT NULL",
                [scope_key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(persistence)?;
        Ok(StoreCounts {
            unresolved_unknown_count: to_u64(unresolved_unknown_count)?,
            linked_open_order_count: to_u64(linked_open_order_count)?,
            linked_active_stop_count: to_u64(linked_active_stop_count)?,
        })
    }
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let mut version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(persistence)?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSchema(version));
    }
    if version == 0 {
        let transaction = connection.transaction().map_err(persistence)?;
        transaction
            .execute_batch(MIGRATION_V1)
            .map_err(persistence)?;
        transaction
            .pragma_update(None, "user_version", 1)
            .map_err(persistence)?;
        transaction.commit().map_err(persistence)?;
        version = 1;
    }
    if version == 1 {
        let transaction = connection.transaction().map_err(persistence)?;
        transaction
            .execute_batch(MIGRATION_V2)
            .map_err(persistence)?;
        transaction
            .pragma_update(None, "user_version", 2)
            .map_err(persistence)?;
        transaction.commit().map_err(persistence)?;
        version = 2;
    }
    if version == 2 {
        let transaction = connection.transaction().map_err(persistence)?;
        transaction
            .execute_batch(MIGRATION_V3)
            .map_err(persistence)?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(persistence)?;
        transaction.commit().map_err(persistence)?;
    }
    Ok(())
}

fn load_mutation(
    connection: &Connection,
    logical_request_id: &str,
) -> Result<Option<MutationRecord>, StoreError> {
    connection
        .query_row(
            "SELECT logical_request_id, scope_key, mutation_kind, state,
                redacted_request_evidence, broker_evidence_ref, created_at_unix_ms,
                updated_at_unix_ms, correlation_id, reconciliation_disposition,
                runtime_epoch
             FROM mutation_journal WHERE logical_request_id=?1",
            [logical_request_id],
            mutation_from_row,
        )
        .optional()
        .map_err(persistence)
}

fn mutation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MutationRecord> {
    MutationRecord {
        logical_request_id: row.get(0)?,
        scope_key: row.get(1)?,
        kind: decode_sql(row.get::<_, String>(2)?)?,
        state: decode_sql(row.get::<_, String>(3)?)?,
        request_evidence: decode_sql(row.get::<_, String>(4)?)?,
        broker_evidence_ref: row.get(5)?,
        created_at_unix_ms: row.get(6)?,
        updated_at_unix_ms: row.get(7)?,
        correlation_id: row.get(8)?,
        reconciliation_disposition: row
            .get::<_, Option<String>>(9)?
            .map(decode_sql)
            .transpose()?,
        runtime_epoch: to_u64_sql(row.get(10)?)?,
    }
    .validated()
    .map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn upsert_links(
    transaction: &Transaction<'_>,
    scope_key: &str,
    links: &BrokerIdentityLinks,
    runtime_epoch: u64,
    now_unix_ms: i64,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "INSERT INTO broker_identity_links (
                logical_request_id, scope_key, broker_order_id,
                replacement_broker_order_id, broker_stop_order_id,
                updated_at_unix_ms, runtime_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(logical_request_id) DO UPDATE SET
                broker_order_id=COALESCE(excluded.broker_order_id, broker_identity_links.broker_order_id),
                replacement_broker_order_id=COALESCE(excluded.replacement_broker_order_id, broker_identity_links.replacement_broker_order_id),
                broker_stop_order_id=COALESCE(excluded.broker_stop_order_id, broker_identity_links.broker_stop_order_id),
                updated_at_unix_ms=excluded.updated_at_unix_ms,
                runtime_epoch=excluded.runtime_epoch",
            params![
                links.logical_request_id,
                scope_key,
                links.broker_order_id,
                links.replacement_broker_order_id,
                links.broker_stop_order_id,
                now_unix_ms,
                to_i64(runtime_epoch)?,
            ],
        )
        .map_err(|error| {
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                StoreError::Corrupt("conflicting typed broker identity link".into())
            } else {
                persistence(error)
            }
        })?;
    for operation_id in &links.provider_operation_ids {
        transaction
            .execute(
                "INSERT OR IGNORE INTO provider_operation_links
                 (scope_key, logical_request_id, provider_operation_id) VALUES (?1, ?2, ?3)",
                params![scope_key, links.logical_request_id, operation_id],
            )
            .map_err(persistence)?;
    }
    for fill_id in &links.broker_fill_ids {
        transaction
            .execute(
                "INSERT OR IGNORE INTO broker_fill_links
                 (scope_key, logical_request_id, broker_fill_id) VALUES (?1, ?2, ?3)",
                params![scope_key, links.logical_request_id, fill_id],
            )
            .map_err(persistence)?;
    }
    Ok(())
}

fn insert_audit(connection: &Connection, record: &RuntimeAuditRecord) -> Result<(), StoreError> {
    if record.redacted_detail.len() > 2_048 {
        return Err(StoreError::Persistence(
            "audit detail exceeds bounded size".into(),
        ));
    }
    connection
        .execute(
            "INSERT INTO runtime_audit (
                scope_key, runtime_epoch, event_type, reason_code,
                correlation_id, redacted_detail, observed_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.scope_key,
                to_i64(record.runtime_epoch)?,
                record.event_type,
                encode(&record.reason_code)?,
                record.correlation_id,
                record.redacted_detail,
                record.observed_at_unix_ms,
            ],
        )
        .map(|_| ())
        .map_err(persistence)
}

fn verify_epoch_connection(
    connection: &Connection,
    scope_key: &str,
    runtime_epoch: u64,
) -> Result<(), StoreError> {
    let active = connection
        .query_row(
            "SELECT count(*) FROM runtime_instance
             WHERE scope_key=?1 AND epoch=?2 AND lease_active=1",
            params![scope_key, to_i64(runtime_epoch)?],
            |row| row.get::<_, i64>(0),
        )
        .map_err(persistence)?;
    if active == 1 {
        Ok(())
    } else {
        Err(StoreError::StaleEpoch)
    }
}

fn verify_epoch_transaction(
    transaction: &Transaction<'_>,
    scope_key: &str,
    runtime_epoch: u64,
) -> Result<(), StoreError> {
    verify_epoch_connection(transaction, scope_key, runtime_epoch)
}

fn string_set(
    connection: &Connection,
    sql: &str,
    scope_key: &str,
    logical_request_id: &str,
) -> Result<std::collections::BTreeSet<String>, StoreError> {
    let mut statement = connection.prepare(sql).map_err(persistence)?;
    statement
        .query_map(params![scope_key, logical_request_id], |row| row.get(0))
        .map_err(persistence)?
        .map(|row| row.map_err(persistence))
        .collect()
}

fn changed_one(changed: usize) -> Result<(), StoreError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(StoreError::StaleEpoch)
    }
}

fn encode<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|error| StoreError::Persistence(error.to_string()))
}

fn decode_sql<T: DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn to_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Corrupt("integer exceeds SQLite i64".into()))
}

fn to_u64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt("negative persisted integer".into()))
}

fn to_u64_sql(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn persistence(error: impl core::fmt::Display) -> StoreError {
    StoreError::Persistence(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::model::{MutationEvidence, MutationKind, OpaqueRef, Provider, RuntimeEnvironment};

    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vox-runtime-{name}-{}.sqlite",
            uuid::Uuid::new_v4()
        ))
    }

    fn evidence(kind: MutationKind) -> MutationEvidence {
        MutationEvidence {
            command_kind: kind,
            instrument_ref: Some("instrument-1".into()),
            quantity_lots: Some(1),
            price_present: true,
            protection_kind: None,
        }
    }

    fn scope() -> Result<RuntimeScope, crate::model::ModelError> {
        RuntimeScope::new_bound(
            Provider::TInvest,
            RuntimeEnvironment::Sandbox,
            "vox-account-1",
            "account-1",
            OpaqueRef::new("connection:1")?,
            OpaqueRef::new("credential:1")?,
        )
    }

    #[test]
    fn sqlite_uses_wal_full_foreign_keys_and_migrates() -> Result<(), Box<dyn std::error::Error>> {
        let path = path("config");
        let store = SqliteRuntimeStore::open(&path)?;
        let config = store.configuration()?;
        assert_eq!(config.journal_mode.to_ascii_lowercase(), "wal");
        assert!(config.foreign_keys);
        assert_eq!(config.synchronous, 2);
        assert_eq!(config.user_version, SCHEMA_VERSION);
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }

    #[test]
    fn second_store_fails_single_writer_lock() -> Result<(), Box<dyn std::error::Error>> {
        let path = path("ownership");
        let first = SqliteRuntimeStore::open(&path)?;
        assert!(matches!(
            SqliteRuntimeStore::open(&path),
            Err(StoreError::OwnershipUnavailable)
        ));
        drop(first);
        let second = SqliteRuntimeStore::open(&path)?;
        drop(second);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }

    #[test]
    fn epoch_fences_old_work_and_unknown_survives_restart() -> Result<(), Box<dyn std::error::Error>>
    {
        let path = path("epoch");
        let scope = scope()?;
        let first = SqliteRuntimeStore::open(&path)?;
        let first_epoch = first.acquire_ownership(&scope, "owner-1", 1)?;
        let record = MutationRecord::prepared(
            &scope,
            "request-1",
            MutationKind::PostOrder,
            evidence(MutationKind::PostOrder),
            "correlation-1",
            first_epoch,
            1,
        )?;
        first.insert_mutation(&record)?;
        first.claim_dispatch_unknown(&scope.key(), "request-1", first_epoch, 2)?;
        drop(first);

        let restarted = SqliteRuntimeStore::open(&path)?;
        let second_epoch = restarted.acquire_ownership(&scope, "owner-2", 3)?;
        assert!(second_epoch > first_epoch);
        assert_eq!(
            restarted.verify_epoch(&scope.key(), first_epoch),
            Err(StoreError::StaleEpoch)
        );
        let unresolved = restarted.unresolved_mutations(&scope.key())?;
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].state, JournalState::UnknownAfterDispatch);
        drop(restarted);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }

    #[test]
    fn logical_request_identity_is_globally_unique() -> Result<(), Box<dyn std::error::Error>> {
        let path = path("unique");
        let scope = scope()?;
        let store = SqliteRuntimeStore::open(&path)?;
        let epoch = store.acquire_ownership(&scope, "owner", 1)?;
        let record = MutationRecord::prepared(
            &scope,
            "request-unique",
            MutationKind::PostOrder,
            evidence(MutationKind::PostOrder),
            "correlation",
            epoch,
            1,
        )?;
        store.insert_mutation(&record)?;
        assert_eq!(
            store.insert_mutation(&record),
            Err(StoreError::DuplicateMutation)
        );
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }

    #[test]
    fn v2_to_v3_migration_preserves_unknown_without_legacy_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = path("migration-unknown");
        let scope = scope()?;
        {
            let store = SqliteRuntimeStore::open(&path)?;
            let epoch = store.acquire_ownership(&scope, "owner", 1)?;
            let record = MutationRecord::prepared(
                &scope,
                "migration-unknown",
                MutationKind::PostOrder,
                evidence(MutationKind::PostOrder),
                "migration-correlation",
                epoch,
                1,
            )?;
            store.insert_mutation(&record)?;
            store.claim_dispatch_unknown(&scope.key(), "migration-unknown", epoch, 2)?;
        }
        {
            let connection = Connection::open(&path)?;
            connection.execute_batch(
                "UPDATE mutation_journal
                     SET redacted_request_evidence='quantity=1; token=must-not-survive';
                 PRAGMA user_version=2;",
            )?;
        }
        let migrated = SqliteRuntimeStore::open(&path)?;
        assert_eq!(migrated.configuration()?.user_version, SCHEMA_VERSION);
        let unresolved = migrated.unresolved_mutations(&scope.key())?;
        assert_eq!(unresolved.len(), 1);
        assert_eq!(
            unresolved[0].request_evidence,
            MutationEvidence {
                command_kind: MutationKind::PostOrder,
                instrument_ref: None,
                quantity_lots: None,
                price_present: false,
                protection_kind: None,
            }
        );
        let stored_evidence: String = migrated.connection()?.query_row(
            "SELECT redacted_request_evidence FROM mutation_journal
             WHERE logical_request_id='migration-unknown'",
            [],
            |row| row.get(0),
        )?;
        assert!(!stored_evidence.contains("must-not-survive"));
        drop(migrated);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }

    #[test]
    fn post_and_cancel_mutations_can_link_same_broker_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = path("shared-broker-identity");
        let scope = scope()?;
        let store = SqliteRuntimeStore::open(&path)?;
        let epoch = store.acquire_ownership(&scope, "owner", 1)?;
        for (id, kind) in [
            ("post-request", MutationKind::PostStopOrder),
            ("cancel-request", MutationKind::CancelStopOrder),
        ] {
            store.insert_mutation(&MutationRecord::prepared(
                &scope,
                id,
                kind,
                evidence(kind),
                format!("correlation-{id}"),
                epoch,
                1,
            )?)?;
            store.upsert_identity_links(
                &scope.key(),
                &BrokerIdentityLinks {
                    logical_request_id: id.into(),
                    broker_stop_order_id: Some("shared-stop-id".into()),
                    ..BrokerIdentityLinks::default()
                },
                epoch,
                2,
            )?;
        }
        assert_eq!(store.all_identity_links(&scope.key())?.len(), 2);
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }

    #[test]
    fn newer_schema_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let path = path("newer");
        let connection = Connection::open(&path)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION + 1)?;
        drop(connection);
        assert!(matches!(
            SqliteRuntimeStore::open(&path),
            Err(StoreError::UnsupportedSchema(version)) if version == SCHEMA_VERSION + 1
        ));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("runtime.lock"));
        Ok(())
    }
}
