use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use vox_domain::{
    AuthoritativeMutationOutcome, BrokerOrderId, ClientRequestId, ExchangeOrderId,
    MutationDecision, MutationEvidence, MutationEvidenceStore, MutationOutcome, MutationRecovery,
    StoreError,
};

const CHILD_PATH: &str = "VOX_RESTART_CHILD_PATH";
const CHILD_REQUEST_ID: &str = "VOX_RESTART_CHILD_REQUEST_ID";
const CHILD_EXPECTED_STATE: &str = "VOX_RESTART_CHILD_EXPECTED_STATE";
const CHILD_BROKER_ID: &str = "VOX_RESTART_CHILD_BROKER_ID";

struct FileEvidenceStore {
    path: PathBuf,
}

impl FileEvidenceStore {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn write_to(file: &mut File, evidence: &MutationEvidence) -> Result<(), StoreError> {
        let mut bytes = serde_json::to_vec(evidence).map_err(store_error)?;
        bytes.push(b'\n');
        file.write_all(&bytes).map_err(store_error)?;
        file.sync_all().map_err(store_error)
    }
}

impl MutationEvidenceStore for FileEvidenceStore {
    fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path).map_err(store_error)?;
        let complete_length = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        if complete_length == 0 {
            return Err(StoreError(
                "evidence journal has no complete durable record".into(),
            ));
        }

        let mut latest = None;
        for line in bytes[..complete_length].split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let evidence: MutationEvidence = serde_json::from_slice(line).map_err(store_error)?;
            if evidence.client_request_id() == id {
                latest = Some(evidence);
            }
        }
        Ok(latest)
    }

    fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(store_error)?;
        Self::write_to(&mut file, evidence)
    }

    fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError> {
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                Self::write_to(&mut file, evidence)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.load(evidence.client_request_id())?;
                if existing.as_ref().map(MutationEvidence::outcome)
                    == Some(MutationOutcome::NotDispatched)
                {
                    self.persist(evidence)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(error) => Err(store_error(error)),
        }
    }

    fn resolve_unknown(
        &mut self,
        expected: &MutationEvidence,
        resolved: &MutationEvidence,
    ) -> Result<bool, StoreError> {
        let lock_path = self.path.with_extension("resolve.lock");
        let lock = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(lock) => lock,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => return Err(store_error(error)),
        };
        let result = (|| {
            if self.load(expected.client_request_id())?.as_ref() == Some(expected) {
                self.persist(resolved).map(|()| true)
            } else {
                Ok(false)
            }
        })();
        drop(lock);
        let cleanup = fs::remove_file(lock_path).map_err(store_error);
        match (result, cleanup) {
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
            (Ok(resolved), Ok(())) => Ok(resolved),
        }
    }
}

fn store_error(error: impl std::fmt::Display) -> StoreError {
    StoreError(error.to_string())
}

struct TestFile(PathBuf);

impl TestFile {
    fn unique() -> Result<Self, std::time::SystemTimeError> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        Ok(Self(std::env::temp_dir().join(format!(
            "vox-domain-restart-{}-{nonce}.json",
            std::process::id()
        ))))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        match fs::remove_file(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove test evidence: {error}"),
        }
    }
}

fn run_restart_child(
    path: &Path,
    request_id: &ClientRequestId,
    expected_state: &str,
    broker_id: Option<&BrokerOrderId>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "restart_child_recovers_persisted_identity_without_resubmit",
            "--ignored",
        ])
        .env(CHILD_PATH, path)
        .env(CHILD_REQUEST_ID, request_id.as_str())
        .env(CHILD_EXPECTED_STATE, expected_state);
    if let Some(broker_id) = broker_id {
        command.env(CHILD_BROKER_ID, broker_id.as_str());
    }

    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "restart child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}

#[test]
fn fresh_process_recovers_unknown_and_terminal_identity_without_resubmitting()
-> Result<(), Box<dyn std::error::Error>> {
    let file = TestFile::unique()?;
    let request_id = ClientRequestId::new("durable-request-1")?;

    {
        let mut first_process =
            MutationRecovery::new(FileEvidenceStore::new(file.path().to_path_buf()));
        assert_eq!(
            first_process.decision(&request_id)?,
            MutationDecision::Submit
        );
        first_process
            .persist_before_dispatch(request_id.clone(), Some("transport-correlation-1".into()))?;
    }

    run_restart_child(file.path(), &request_id, "unknown", None)?;

    let broker_id = BrokerOrderId::new("broker-order-1")?;
    {
        let reconciler = MutationRecovery::new(FileEvidenceStore::new(file.path().to_path_buf()));
        let evidence = reconciler
            .into_store()
            .load(&request_id)?
            .ok_or_else(|| StoreError("persisted UNKNOWN evidence missing".into()))?
            .with_broker_order_id(broker_id.clone())
            .with_exchange_order_id(ExchangeOrderId::new("exchange-order-1")?);

        let store = FileEvidenceStore::new(file.path().to_path_buf());
        let mut reconciler = MutationRecovery::new(store);
        reconciler
            .persist_authoritative_outcome(evidence, AuthoritativeMutationOutcome::Accepted)?;
    }

    run_restart_child(file.path(), &request_id, "accepted", Some(&broker_id))?;
    Ok(())
}

#[test]
#[ignore = "spawned by fresh_process_recovers_unknown_and_terminal_identity_without_resubmitting"]
fn restart_child_recovers_persisted_identity_without_resubmit()
-> Result<(), Box<dyn std::error::Error>> {
    let path = env::var_os(CHILD_PATH)
        .map(PathBuf::from)
        .ok_or_else(|| StoreError(format!("{CHILD_PATH} is required")))?;
    let request_id = ClientRequestId::new(env::var(CHILD_REQUEST_ID)?)?;
    let expected_state = env::var(CHILD_EXPECTED_STATE)?;
    let recovery = MutationRecovery::new(FileEvidenceStore::new(path));
    let evidence = recovery
        .into_store()
        .load(&request_id)?
        .ok_or_else(|| StoreError("restart child could not load evidence".into()))?;

    assert_eq!(evidence.client_request_id(), &request_id);
    let recovery = MutationRecovery::new(FileEvidenceStore::new(
        env::var_os(CHILD_PATH)
            .map(PathBuf::from)
            .ok_or_else(|| StoreError(format!("{CHILD_PATH} is required")))?,
    ));
    match expected_state.as_str() {
        "unknown" => {
            assert_eq!(evidence.outcome(), MutationOutcome::Unknown);
            assert_eq!(evidence.correlation_id(), Some("transport-correlation-1"));
            assert_eq!(recovery.decision(&request_id)?, MutationDecision::Reconcile);
        }
        "accepted" => {
            assert_eq!(evidence.outcome(), MutationOutcome::Accepted);
            assert_eq!(
                evidence.broker_order_id().map(BrokerOrderId::as_str),
                Some(env::var(CHILD_BROKER_ID)?.as_str())
            );
            assert_eq!(
                recovery.decision(&request_id)?,
                MutationDecision::DoNotSubmit
            );
        }
        value => return Err(StoreError(format!("unexpected child state {value}")).into()),
    }
    assert_ne!(recovery.decision(&request_id)?, MutationDecision::Submit);
    Ok(())
}
