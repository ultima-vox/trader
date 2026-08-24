//! Canonical operation history plus exhaustive provider-cursor traversal.

use std::collections::HashSet;

use async_trait::async_trait;
use thiserror::Error;
use vox_domain::UnitsNano;

use crate::account::{
    AccountDataError, ProviderTimestamp, optional_money, optional_quotation, optional_text,
    optional_timestamp,
};
use crate::canonical::CanonicalMoney;
use crate::generated::v1;
use crate::{GrpcError, TInvestGrpcClient};

pub const DEFAULT_OPERATIONS_PAGE_SIZE: i32 = 100;
pub const MIN_OPERATIONS_PAGE_SIZE: i32 = 3;
pub const MAX_OPERATIONS_PAGE_SIZE: i32 = 1_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationsFilter {
    pub account_id: String,
    pub instrument_id: Option<String>,
    pub from: Option<ProviderTimestamp>,
    pub to: Option<ProviderTimestamp>,
    pub initial_cursor: Option<String>,
    pub page_size: Option<i32>,
    pub operation_types: Vec<i32>,
    pub state: Option<i32>,
    pub without_commissions: Option<bool>,
    pub without_trades: Option<bool>,
    pub without_overnights: Option<bool>,
}

impl OperationsFilter {
    fn provider_request(&self, cursor: Option<String>) -> v1::GetOperationsByCursorRequest {
        v1::GetOperationsByCursorRequest {
            account_id: self.account_id.clone(),
            instrument_id: self.instrument_id.clone(),
            from: self.from.map(provider_timestamp),
            to: self.to.map(provider_timestamp),
            cursor,
            limit: self.page_size,
            operation_types: self.operation_types.clone(),
            state: self.state,
            without_commissions: self.without_commissions,
            without_trades: self.without_trades,
            without_overnights: self.without_overnights,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyOperationRecord {
    pub provider_operation_id: Option<String>,
    pub parent_operation_id: Option<String>,
    pub currency: Option<String>,
    pub payment: Option<CanonicalMoney>,
    pub price: Option<CanonicalMoney>,
    pub state: i32,
    pub quantity: i64,
    pub quantity_rest: i64,
    pub figi: Option<String>,
    pub instrument_type: Option<String>,
    pub date: Option<ProviderTimestamp>,
    pub description: Option<String>,
    pub operation_type: i32,
    pub asset_uid: Option<String>,
    pub position_uid: Option<String>,
    pub instrument_uid: Option<String>,
    pub trades: Vec<LegacyOperationTrade>,
    pub child_operations: Vec<CanonicalChildOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyOperationTrade {
    pub trade_id: Option<String>,
    pub date: Option<ProviderTimestamp>,
    pub quantity: i64,
    pub price: Option<CanonicalMoney>,
}

impl TryFrom<v1::Operation> for LegacyOperationRecord {
    type Error = AccountDataError;

    fn try_from(value: v1::Operation) -> Result<Self, Self::Error> {
        Ok(Self {
            provider_operation_id: optional_text(value.id),
            parent_operation_id: optional_text(value.parent_operation_id),
            currency: optional_text(value.currency),
            payment: optional_money(value.payment)?,
            price: optional_money(value.price)?,
            state: value.state,
            quantity: value.quantity,
            quantity_rest: value.quantity_rest,
            figi: optional_text(value.figi),
            instrument_type: optional_text(value.instrument_type),
            date: optional_timestamp(value.date)?,
            description: optional_text(value.r#type),
            operation_type: value.operation_type,
            asset_uid: optional_text(value.asset_uid),
            position_uid: optional_text(value.position_uid),
            instrument_uid: optional_text(value.instrument_uid),
            trades: value
                .trades
                .into_iter()
                .map(|trade| {
                    Ok(LegacyOperationTrade {
                        trade_id: optional_text(trade.trade_id),
                        date: optional_timestamp(trade.date_time)?,
                        quantity: trade.quantity,
                        price: optional_money(trade.price)?,
                    })
                })
                .collect::<Result<_, AccountDataError>>()?,
            child_operations: value
                .child_operations
                .into_iter()
                .map(|child| {
                    Ok(CanonicalChildOperation {
                        instrument_uid: optional_text(child.instrument_uid),
                        payment: optional_money(child.payment)?,
                    })
                })
                .collect::<Result<_, AccountDataError>>()?,
        })
    }
}

pub fn canonical_legacy_operations(
    response: v1::OperationsResponse,
) -> Result<Vec<LegacyOperationRecord>, AccountDataError> {
    response
        .operations
        .into_iter()
        .map(LegacyOperationRecord::try_from)
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOperation {
    /// Provider pagination identity. Stable only for traversal/re-import boundaries.
    pub cursor: Option<String>,
    pub broker_account_id: Option<String>,
    /// Provider explicitly documents this value as mutable. Never use as durable storage key.
    pub provider_operation_id: Option<String>,
    /// Parent ID is also mutable when provider changes parent operation ID.
    pub parent_operation_id: Option<String>,
    pub name: Option<String>,
    pub date: Option<ProviderTimestamp>,
    pub operation_type: i32,
    pub description: Option<String>,
    pub state: i32,
    pub instrument_uid: Option<String>,
    pub figi: Option<String>,
    pub instrument_type: Option<String>,
    pub instrument_kind: i32,
    pub position_uid: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub asset_uid: Option<String>,
    pub payment: Option<CanonicalMoney>,
    pub price: Option<CanonicalMoney>,
    pub commission: Option<CanonicalMoney>,
    pub yield_amount: Option<CanonicalMoney>,
    pub yield_relative: Option<UnitsNano>,
    pub accrued_interest: Option<CanonicalMoney>,
    pub quantity: i64,
    pub quantity_rest: i64,
    pub quantity_done: i64,
    pub cancelled_at: Option<ProviderTimestamp>,
    pub cancel_reason: Option<String>,
    pub trades: Vec<CanonicalOperationTrade>,
    pub child_operations: Vec<CanonicalChildOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOperationTrade {
    pub trade_id: Option<String>,
    pub date: Option<ProviderTimestamp>,
    pub quantity: i64,
    pub price: Option<CanonicalMoney>,
    pub yield_amount: Option<CanonicalMoney>,
    pub yield_relative: Option<UnitsNano>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalChildOperation {
    pub instrument_uid: Option<String>,
    pub payment: Option<CanonicalMoney>,
}

impl TryFrom<v1::OperationItem> for CanonicalOperation {
    type Error = AccountDataError;

    fn try_from(value: v1::OperationItem) -> Result<Self, Self::Error> {
        let trades = value
            .trades_info
            .map(|trades| trades.trades)
            .unwrap_or_default()
            .into_iter()
            .map(CanonicalOperationTrade::try_from)
            .collect::<Result<_, _>>()?;
        let child_operations = value
            .child_operations
            .into_iter()
            .map(|child| {
                Ok(CanonicalChildOperation {
                    instrument_uid: optional_text(child.instrument_uid),
                    payment: optional_money(child.payment)?,
                })
            })
            .collect::<Result<_, AccountDataError>>()?;
        Ok(Self {
            cursor: optional_text(value.cursor),
            broker_account_id: optional_text(value.broker_account_id),
            provider_operation_id: optional_text(value.id),
            parent_operation_id: optional_text(value.parent_operation_id),
            name: optional_text(value.name),
            date: optional_timestamp(value.date)?,
            operation_type: value.r#type,
            description: optional_text(value.description),
            state: value.state,
            instrument_uid: optional_text(value.instrument_uid),
            figi: optional_text(value.figi),
            instrument_type: optional_text(value.instrument_type),
            instrument_kind: value.instrument_kind,
            position_uid: optional_text(value.position_uid),
            ticker: optional_text(value.ticker),
            class_code: optional_text(value.class_code),
            asset_uid: optional_text(value.asset_uid),
            payment: optional_money(value.payment)?,
            price: optional_money(value.price)?,
            commission: optional_money(value.commission)?,
            yield_amount: optional_money(value.r#yield)?,
            yield_relative: optional_quotation(value.yield_relative)?,
            accrued_interest: optional_money(value.accrued_int)?,
            quantity: value.quantity,
            quantity_rest: value.quantity_rest,
            quantity_done: value.quantity_done,
            cancelled_at: optional_timestamp(value.cancel_date_time)?,
            cancel_reason: optional_text(value.cancel_reason),
            trades,
            child_operations,
        })
    }
}

impl TryFrom<v1::OperationItemTrade> for CanonicalOperationTrade {
    type Error = AccountDataError;

    fn try_from(value: v1::OperationItemTrade) -> Result<Self, Self::Error> {
        Ok(Self {
            trade_id: optional_text(value.num),
            date: optional_timestamp(value.date)?,
            quantity: value.quantity,
            price: optional_money(value.price)?,
            yield_amount: optional_money(value.r#yield)?,
            yield_relative: optional_quotation(value.yield_relative)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationHistory {
    pub items: Vec<CanonicalOperation>,
    pub pages: usize,
}

#[derive(Clone, Debug)]
pub struct OperationsPaginator {
    filter: OperationsFilter,
    max_pages: usize,
}

impl OperationsPaginator {
    pub fn new(mut filter: OperationsFilter) -> Result<Self, PaginationError> {
        if filter.account_id.trim().is_empty() {
            return Err(PaginationError::MissingAccountId);
        }
        if filter
            .from
            .zip(filter.to)
            .is_some_and(|(from, to)| from > to)
        {
            return Err(PaginationError::InvalidRange);
        }
        let limit = filter.page_size.unwrap_or(DEFAULT_OPERATIONS_PAGE_SIZE);
        if !(MIN_OPERATIONS_PAGE_SIZE..=MAX_OPERATIONS_PAGE_SIZE).contains(&limit) {
            return Err(PaginationError::InvalidPageSize(limit));
        }
        filter.page_size = Some(limit);
        Ok(Self {
            filter,
            max_pages: 10_000,
        })
    }

    #[must_use]
    pub fn with_max_pages(mut self, max_pages: usize) -> Self {
        self.max_pages = max_pages.max(1);
        self
    }

    pub async fn collect(
        &self,
        client: &TInvestGrpcClient,
    ) -> Result<OperationHistory, PaginationFailure> {
        self.collect_from(&ProductionOperationsPageSource(client))
            .await
    }

    pub async fn collect_sandbox(
        &self,
        client: &TInvestGrpcClient,
    ) -> Result<OperationHistory, PaginationFailure> {
        self.collect_from(&SandboxOperationsPageSource(client))
            .await
    }

    async fn collect_from<S: OperationsPageSource + ?Sized>(
        &self,
        source: &S,
    ) -> Result<OperationHistory, PaginationFailure> {
        let mut completed = OperationHistory {
            items: Vec::new(),
            pages: 0,
        };
        let mut cursor = self.filter.initial_cursor.clone();
        let mut seen = HashSet::new();
        if let Some(initial) = cursor.as_ref() {
            seen.insert(initial.clone());
        }

        loop {
            if completed.pages >= self.max_pages {
                return Err(PaginationFailure {
                    completed,
                    cause: PaginationFailureCause::Malformed(PaginationError::PageBoundExceeded),
                });
            }
            let request = self.filter.provider_request(cursor.clone());
            let response = source
                .page(request)
                .await
                .map_err(|error| PaginationFailure {
                    completed: completed.clone(),
                    cause: PaginationFailureCause::Provider(error),
                })?;
            completed.pages += 1;
            for item in response.items {
                let operation =
                    CanonicalOperation::try_from(item).map_err(|error| PaginationFailure {
                        completed: completed.clone(),
                        cause: PaginationFailureCause::Canonical(error),
                    })?;
                if operation.broker_account_id.as_deref() != Some(self.filter.account_id.as_str()) {
                    return Err(PaginationFailure {
                        completed,
                        cause: PaginationFailureCause::Canonical(
                            AccountDataError::IdentityMismatch("operation.broker_account_id"),
                        ),
                    });
                }
                completed.items.push(operation);
            }

            let next = optional_text(response.next_cursor);
            match (response.has_next, next) {
                (false, None) => return Ok(completed),
                (true, None) => {
                    return Err(PaginationFailure {
                        completed,
                        cause: PaginationFailureCause::Malformed(
                            PaginationError::MissingContinuationCursor,
                        ),
                    });
                }
                (false, Some(_)) => {
                    return Err(PaginationFailure {
                        completed,
                        cause: PaginationFailureCause::Malformed(
                            PaginationError::ContradictoryContinuation,
                        ),
                    });
                }
                (true, Some(next)) => {
                    if !seen.insert(next.clone()) {
                        return Err(PaginationFailure {
                            completed,
                            cause: PaginationFailureCause::Malformed(PaginationError::CursorCycle),
                        });
                    }
                    cursor = Some(next);
                }
            }
        }
    }
}

#[async_trait]
trait OperationsPageSource: Send + Sync {
    async fn page(
        &self,
        request: v1::GetOperationsByCursorRequest,
    ) -> Result<v1::GetOperationsByCursorResponse, GrpcError>;
}

struct ProductionOperationsPageSource<'a>(&'a TInvestGrpcClient);

#[async_trait]
impl OperationsPageSource for ProductionOperationsPageSource<'_> {
    async fn page(
        &self,
        request: v1::GetOperationsByCursorRequest,
    ) -> Result<v1::GetOperationsByCursorResponse, GrpcError> {
        self.0
            .get_operations_by_cursor(request)
            .await
            .map(|response| response.body)
    }
}

/// Explicit sandbox route; prevents accidentally calling production history method for sandbox parity.
struct SandboxOperationsPageSource<'a>(&'a TInvestGrpcClient);

#[async_trait]
impl OperationsPageSource for SandboxOperationsPageSource<'_> {
    async fn page(
        &self,
        request: v1::GetOperationsByCursorRequest,
    ) -> Result<v1::GetOperationsByCursorResponse, GrpcError> {
        self.0
            .get_sandbox_operations_by_cursor(request)
            .await
            .map(|response| response.body)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PaginationError {
    #[error("operations cursor account_id is required")]
    MissingAccountId,
    #[error("operations cursor start must not follow end")]
    InvalidRange,
    #[error("operations cursor page size {0} is outside 3..=1000")]
    InvalidPageSize(i32),
    #[error("provider set has_next but omitted next_cursor")]
    MissingContinuationCursor,
    #[error("provider returned next_cursor while has_next is false")]
    ContradictoryContinuation,
    #[error("provider repeated an operations cursor")]
    CursorCycle,
    #[error("operations pagination exceeded configured page bound")]
    PageBoundExceeded,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PaginationFailureCause {
    #[error("{0}")]
    Provider(GrpcError),
    #[error("{0}")]
    Canonical(AccountDataError),
    #[error("{0}")]
    Malformed(PaginationError),
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("operations pagination failed after {pages} complete page(s): {cause}", pages = completed.pages)]
pub struct PaginationFailure {
    pub completed: OperationHistory,
    pub cause: PaginationFailureCause,
}

fn provider_timestamp(value: ProviderTimestamp) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: value.seconds,
        nanos: value.nanos,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::{GrpcErrorKind, GrpcRequestMetadata};
    use uuid::Uuid;

    struct StubSource(Mutex<VecDeque<Result<v1::GetOperationsByCursorResponse, GrpcError>>>);

    #[async_trait]
    impl OperationsPageSource for StubSource {
        async fn page(
            &self,
            _request: v1::GetOperationsByCursorRequest,
        ) -> Result<v1::GetOperationsByCursorResponse, GrpcError> {
            self.0
                .lock()
                .expect("stub lock")
                .pop_front()
                .expect("planned page")
        }
    }

    fn request() -> OperationsFilter {
        OperationsFilter {
            account_id: "account".to_owned(),
            instrument_id: None,
            from: None,
            to: None,
            initial_cursor: None,
            page_size: Some(3),
            operation_types: Vec::new(),
            state: None,
            without_commissions: None,
            without_trades: None,
            without_overnights: None,
        }
    }

    fn item(id: &str) -> v1::OperationItem {
        v1::OperationItem {
            id: id.to_owned(),
            broker_account_id: "account".to_owned(),
            ..Default::default()
        }
    }

    fn source(pages: Vec<Result<v1::GetOperationsByCursorResponse, GrpcError>>) -> StubSource {
        StubSource(Mutex::new(pages.into()))
    }

    #[tokio::test]
    async fn empty_one_and_many_pages_terminate_without_deduplication() {
        let empty = source(vec![Ok(v1::GetOperationsByCursorResponse::default())]);
        let history = OperationsPaginator::new(request())
            .expect("paginator")
            .collect_from(&empty)
            .await
            .expect("empty is legal");
        assert_eq!(
            history,
            OperationHistory {
                items: Vec::new(),
                pages: 1
            }
        );

        let many = source(vec![
            Ok(v1::GetOperationsByCursorResponse {
                has_next: true,
                next_cursor: "next".to_owned(),
                items: vec![item("mutable")],
            }),
            Ok(v1::GetOperationsByCursorResponse {
                has_next: false,
                next_cursor: String::new(),
                items: vec![item("mutable")],
            }),
        ]);
        let history = OperationsPaginator::new(request())
            .expect("paginator")
            .collect_from(&many)
            .await
            .expect("many pages");
        assert_eq!(history.pages, 2);
        assert_eq!(history.items.len(), 2, "mutable IDs are not dedup keys");
    }

    #[tokio::test]
    async fn cycle_and_contradictory_states_fail_closed() {
        let cycle = source(vec![Ok(v1::GetOperationsByCursorResponse {
            has_next: true,
            next_cursor: "same".to_owned(),
            items: Vec::new(),
        })]);
        let mut cyclic_request = request();
        cyclic_request.initial_cursor = Some("same".to_owned());
        let error = OperationsPaginator::new(cyclic_request)
            .expect("paginator")
            .collect_from(&cycle)
            .await
            .expect_err("cycle");
        assert_eq!(
            error.cause,
            PaginationFailureCause::Malformed(PaginationError::CursorCycle)
        );

        let contradictory = source(vec![Ok(v1::GetOperationsByCursorResponse {
            has_next: false,
            next_cursor: "unexpected".to_owned(),
            items: Vec::new(),
        })]);
        assert!(
            OperationsPaginator::new(request())
                .expect("paginator")
                .collect_from(&contradictory)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn provider_failure_returns_completed_prefix() {
        let provider_error = GrpcError {
            metadata: GrpcRequestMetadata {
                request_id: Uuid::nil(),
                method: "GetOperationsByCursor",
                attempt: 1,
                mutation: false,
            },
            kind: GrpcErrorKind::InvalidAuthorizationMetadata,
        };
        let pages = source(vec![
            Ok(v1::GetOperationsByCursorResponse {
                has_next: true,
                next_cursor: "next".to_owned(),
                items: vec![item("first")],
            }),
            Err(provider_error),
        ]);
        let error = OperationsPaginator::new(request())
            .expect("paginator")
            .collect_from(&pages)
            .await
            .expect_err("partial failure");
        assert_eq!(error.completed.pages, 1);
        assert_eq!(error.completed.items.len(), 1);
    }

    #[test]
    fn page_size_boundaries_match_provider_contract() {
        let mut below = request();
        below.page_size = Some(MIN_OPERATIONS_PAGE_SIZE - 1);
        assert!(matches!(
            OperationsPaginator::new(below),
            Err(PaginationError::InvalidPageSize(2))
        ));
        let mut maximum = request();
        maximum.page_size = Some(MAX_OPERATIONS_PAGE_SIZE);
        assert!(OperationsPaginator::new(maximum).is_ok());
        let mut above = request();
        above.page_size = Some(MAX_OPERATIONS_PAGE_SIZE + 1);
        assert!(OperationsPaginator::new(above).is_err());
    }
}
