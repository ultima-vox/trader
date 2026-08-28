//! Complete sandbox qualification accounting. No capability may be silently skipped.

use std::collections::BTreeMap;

use thiserror::Error;
use vox_domain::{
    ClientRequestId, MutationDecision, MutationEvidence, MutationEvidenceStore, MutationRecovery,
    StoreError,
};

pub const SANDBOX_QUALIFICATION_ROWS: [&str; 21] = [
    "account_discovery_readiness",
    "max_lots",
    "pre_trade_estimate",
    "market_order_lifecycle",
    "limit_order_lifecycle",
    "async_order_lifecycle",
    "order_state_and_list",
    "replace_lifecycle",
    "cancel_lifecycle",
    "broker_idempotency_evidence",
    "ambiguous_dispatch_fault_injection",
    "fixed_stop_only",
    "stop_limit",
    "take_profit_only",
    "trailing_relative",
    "trailing_absolute",
    "fixed_stop_plus_take_profit",
    "trailing_plus_take_profit",
    "trades_stream_health",
    "order_state_stream_health",
    "cleanup_readback",
];

/// Controlled post-dispatch fault harness used by final sandbox runner.
pub fn qualify_ambiguous_dispatch_guard() -> Result<(), StoreError> {
    #[derive(Default)]
    struct FaultStore(BTreeMap<ClientRequestId, MutationEvidence>);
    impl MutationEvidenceStore for FaultStore {
        fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError> {
            Ok(self.0.get(id).cloned())
        }
        fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError> {
            self.0
                .insert(evidence.client_request_id().clone(), evidence.clone());
            Ok(())
        }
        fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError> {
            if self.0.contains_key(evidence.client_request_id()) {
                return Ok(false);
            }
            self.persist(evidence)?;
            Ok(true)
        }
        fn resolve_unknown(
            &mut self,
            expected: &MutationEvidence,
            resolved: &MutationEvidence,
        ) -> Result<bool, StoreError> {
            if self.0.get(expected.client_request_id()) != Some(expected) {
                return Ok(false);
            }
            self.persist(resolved)?;
            Ok(true)
        }
    }

    let id = ClientRequestId::new("issue-10-controlled-ambiguous-dispatch")
        .map_err(|error| StoreError(error.to_string()))?;
    let mut recovery = MutationRecovery::new(FaultStore::default());
    recovery.persist_before_dispatch(id.clone(), Some("fault-injection".into()))?;
    if recovery.decision(&id)? != MutationDecision::Reconcile {
        return Err(StoreError(
            "UNKNOWN_AFTER_DISPATCH did not force reconciliation".into(),
        ));
    }
    match recovery.persist_before_dispatch(id, Some("duplicate".into())) {
        Err(_) => Ok(()),
        Ok(_) => Err(StoreError(
            "ambiguous mutation identity was dispatched twice".into(),
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QualificationEvidence {
    Qualified(String),
    QualifiedWithProviderDeviation(String),
    GatedUnavailable(String),
    ProviderBlocked(String),
    Failed(String),
}

impl QualificationEvidence {
    fn render(&self, row: &str) -> String {
        match self {
            Self::Qualified(detail) => format!("QUALIFIED {row}: {detail}"),
            Self::QualifiedWithProviderDeviation(detail) => {
                format!("QUALIFIED_WITH_PROVIDER_DEVIATION {row}: {detail}")
            }
            Self::GatedUnavailable(reason) => format!("GATED/UNAVAILABLE {row}: {reason}"),
            Self::ProviderBlocked(reason) => format!("BLOCKED/PROVIDER {row}: {reason}"),
            Self::Failed(reason) => format!("FAILED {row}: {reason}"),
        }
    }
}

#[derive(Default)]
pub struct SandboxQualificationLedger {
    evidence: BTreeMap<&'static str, QualificationEvidence>,
}

impl SandboxQualificationLedger {
    pub fn record(
        &mut self,
        row: &'static str,
        evidence: QualificationEvidence,
    ) -> Result<(), SandboxQualificationError> {
        if !SANDBOX_QUALIFICATION_ROWS.contains(&row) {
            return Err(SandboxQualificationError::UnknownRow(row.to_owned()));
        }
        if self.evidence.insert(row, evidence).is_some() {
            return Err(SandboxQualificationError::DuplicateRow(row.to_owned()));
        }
        Ok(())
    }

    pub fn lines(&self) -> Result<Vec<String>, SandboxQualificationError> {
        let missing = SANDBOX_QUALIFICATION_ROWS
            .iter()
            .filter(|row| !self.evidence.contains_key(**row))
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(SandboxQualificationError::MissingRows(missing));
        }
        Ok(SANDBOX_QUALIFICATION_ROWS
            .iter()
            .filter_map(|row| self.evidence.get(row).map(|evidence| evidence.render(row)))
            .collect())
    }

    pub fn finish(self) -> Result<Vec<String>, SandboxQualificationError> {
        let lines = self.lines()?;
        let failed = SANDBOX_QUALIFICATION_ROWS
            .iter()
            .filter_map(|row| match self.evidence.get(*row) {
                Some(QualificationEvidence::Failed(reason)) => Some(format!("{row}: {reason}")),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !failed.is_empty() {
            Err(SandboxQualificationError::FailedRows(failed))
        } else {
            let blocked = SANDBOX_QUALIFICATION_ROWS
                .iter()
                .filter_map(|row| match self.evidence.get(*row) {
                    Some(QualificationEvidence::ProviderBlocked(reason)) => {
                        Some(format!("{row}: {reason}"))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if blocked.is_empty() {
                Ok(lines)
            } else {
                Err(SandboxQualificationError::ProviderBlockedRows(blocked))
            }
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SandboxQualificationError {
    #[error("unknown sandbox qualification row: {0}")]
    UnknownRow(String),
    #[error("duplicate sandbox qualification row: {0}")]
    DuplicateRow(String),
    #[error("sandbox qualification omitted rows: {0:?}")]
    MissingRows(Vec<&'static str>),
    #[error("sandbox qualification failed rows: {0:?}")]
    FailedRows(Vec<String>),
    #[error("sandbox qualification blocked by provider rows: {0:?}")]
    ProviderBlockedRows(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_rejects_skip_duplicate_unknown_and_failure() {
        let mut missing = SandboxQualificationLedger::default();
        missing
            .record(
                SANDBOX_QUALIFICATION_ROWS[0],
                QualificationEvidence::Qualified("ok".into()),
            )
            .expect("record");
        assert!(matches!(
            missing.finish(),
            Err(SandboxQualificationError::MissingRows(_))
        ));

        let mut complete = SandboxQualificationLedger::default();
        for row in SANDBOX_QUALIFICATION_ROWS {
            complete
                .record(
                    row,
                    QualificationEvidence::Qualified("contract evidence".into()),
                )
                .expect("unique row");
        }
        let lines = complete.finish().expect("complete");
        assert_eq!(lines.len(), SANDBOX_QUALIFICATION_ROWS.len());
        assert!(lines.iter().all(|line| line.starts_with("QUALIFIED ")));
    }

    #[test]
    fn controlled_ambiguous_dispatch_fault_blocks_duplicate() {
        qualify_ambiguous_dispatch_guard().expect("fault harness");
    }

    #[test]
    fn provider_blocker_is_not_reported_as_implementation_failure() {
        let mut ledger = SandboxQualificationLedger::default();
        for row in SANDBOX_QUALIFICATION_ROWS {
            let evidence = if row == "market_order_lifecycle" {
                QualificationEvidence::ProviderBlocked(
                    "PostSandboxOrder INTERNAL/70001; tracking_id=provider-evidence".into(),
                )
            } else {
                QualificationEvidence::Qualified("contract evidence".into())
            };
            ledger.record(row, evidence).expect("unique row");
        }
        assert!(matches!(
            ledger.finish(),
            Err(SandboxQualificationError::ProviderBlockedRows(rows))
                if rows.len() == 1 && rows[0].contains("70001")
        ));
    }

    #[test]
    fn proven_provider_deviation_is_accepted_but_rendered_distinctly() {
        let mut ledger = SandboxQualificationLedger::default();
        for row in SANDBOX_QUALIFICATION_ROWS {
            let evidence = if row == "trades_stream_health" {
                QualificationEvidence::QualifiedWithProviderDeviation(
                    "matching trade and pings observed; subscription ACK absent".into(),
                )
            } else {
                QualificationEvidence::Qualified("contract evidence".into())
            };
            ledger.record(row, evidence).expect("unique row");
        }
        let lines = ledger.finish().expect("provider deviation is accepted");
        assert!(lines.iter().any(|line| {
            line.starts_with("QUALIFIED_WITH_PROVIDER_DEVIATION trades_stream_health:")
        }));
    }
}
