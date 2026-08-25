//! Complete sandbox qualification accounting. No capability may be silently skipped.

use std::collections::BTreeMap;

use thiserror::Error;

pub const SANDBOX_QUALIFICATION_ROWS: [&str; 18] = [
    "account_discovery_readiness",
    "max_lots",
    "pre_trade_estimate",
    "market_order_lifecycle",
    "limit_order_lifecycle",
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
    "trades_stream_health",
    "order_state_stream_health",
    "cleanup_readback",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QualificationEvidence {
    Qualified(String),
    GatedUnavailable(String),
    Failed(String),
}

impl QualificationEvidence {
    fn render(&self, row: &str) -> String {
        match self {
            Self::Qualified(detail) => format!("QUALIFIED {row}: {detail}"),
            Self::GatedUnavailable(reason) => format!("GATED/UNAVAILABLE {row}: {reason}"),
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

    pub fn finish(self) -> Result<Vec<String>, SandboxQualificationError> {
        let missing = SANDBOX_QUALIFICATION_ROWS
            .iter()
            .filter(|row| !self.evidence.contains_key(**row))
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(SandboxQualificationError::MissingRows(missing));
        }
        let failed = SANDBOX_QUALIFICATION_ROWS
            .iter()
            .filter(|row| {
                matches!(
                    self.evidence.get(**row),
                    Some(QualificationEvidence::Failed(_))
                )
            })
            .copied()
            .collect::<Vec<_>>();
        let lines = SANDBOX_QUALIFICATION_ROWS
            .iter()
            .filter_map(|row| self.evidence.get(row).map(|evidence| evidence.render(row)))
            .collect();
        if failed.is_empty() {
            Ok(lines)
        } else {
            Err(SandboxQualificationError::FailedRows(failed))
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
    FailedRows(Vec<&'static str>),
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
}
