//! Typed report lifecycle. Generation calls are never retried; generated page reads are safe reads.

use thiserror::Error;
use vox_domain::UnitsNano;

use crate::account::{
    AccountDataError, ProviderTimestamp, optional_money, optional_quotation, optional_text,
    optional_timestamp, required_text,
};
use crate::canonical::CanonicalMoney;
use crate::generated::v1;
use crate::{GrpcError, TInvestGrpcClient};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityObservation<T> {
    Qualified(T),
    GatedUnavailable(CapabilityGate),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityGate {
    Permission { provider_code: Option<String> },
    Tariff { provider_code: Option<String> },
    Environment { provider_code: Option<String> },
    AccountKind { provider_code: Option<String> },
}

/// Converts only unambiguous gRPC capability states. Other errors remain failures for callers to
/// investigate; tariff/account-kind gates require a documented provider code and explicit variant.
pub fn observe_documented_gate<T>(
    method: &'static str,
    result: Result<T, GrpcError>,
) -> Result<CapabilityObservation<T>, GrpcError> {
    match result {
        Ok(value) => Ok(CapabilityObservation::Qualified(value)),
        Err(error) => match &error.kind {
            crate::GrpcErrorKind::Provider(provider)
                if matches!(
                    crate::account_qualification::classify_method_gate(method, provider),
                    Some(crate::account_qualification::CapabilityGate::InsufficientPermission)
                ) =>
            {
                Ok(CapabilityObservation::GatedUnavailable(
                    CapabilityGate::Permission {
                        provider_code: Some("40002".to_owned()),
                    },
                ))
            }
            _ => Err(error),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrokerReportRequest {
    Generate {
        account_id: String,
        from: ProviderTimestamp,
        to: ProviderTimestamp,
    },
    Get {
        task_id: String,
        page: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrokerReportState {
    Generating { task_id: String },
    Ready(BrokerReportPage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerReportPage {
    pub task_id: Option<String>,
    pub items_count: i32,
    pub pages_count: i32,
    pub page: i32,
    pub rows: Vec<BrokerReportRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerReportRow {
    pub trade_id: Option<String>,
    pub order_id: Option<String>,
    pub figi: Option<String>,
    pub execute_sign: Option<String>,
    pub trade_at: Option<ProviderTimestamp>,
    pub exchange: Option<String>,
    pub class_code: Option<String>,
    pub direction: Option<String>,
    pub name: Option<String>,
    pub ticker: Option<String>,
    pub price: Option<CanonicalMoney>,
    pub quantity: i64,
    pub order_amount: Option<CanonicalMoney>,
    pub accrued_interest: Option<UnitsNano>,
    pub total_order_amount: Option<CanonicalMoney>,
    pub broker_commission: Option<CanonicalMoney>,
    pub exchange_commission: Option<CanonicalMoney>,
    pub clearing_commission: Option<CanonicalMoney>,
    pub repo_rate: Option<UnitsNano>,
    pub party: Option<String>,
    pub clear_value_at: Option<ProviderTimestamp>,
    pub security_value_at: Option<ProviderTimestamp>,
    pub broker_status: Option<String>,
    pub separate_agreement_type: Option<String>,
    pub separate_agreement_number: Option<String>,
    pub separate_agreement_date: Option<String>,
    pub delivery_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForeignIssuerReportRequest {
    Generate {
        account_id: String,
        from: ProviderTimestamp,
        to: ProviderTimestamp,
    },
    Get {
        task_id: String,
        page: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForeignIssuerReportState {
    Generating { task_id: String },
    Ready(ForeignIssuerReportPage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignIssuerReportPage {
    pub items_count: i32,
    pub pages_count: i32,
    pub page: i32,
    pub rows: Vec<ForeignIssuerReportRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignIssuerReportRow {
    pub record_at: Option<ProviderTimestamp>,
    pub payment_at: Option<ProviderTimestamp>,
    pub security_name: Option<String>,
    pub isin: Option<String>,
    pub issuer_country: Option<String>,
    pub quantity: i64,
    pub dividend: Option<UnitsNano>,
    pub external_commission: Option<UnitsNano>,
    pub dividend_gross: Option<UnitsNano>,
    pub tax: Option<UnitsNano>,
    pub dividend_amount: Option<UnitsNano>,
    pub currency: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportPageTraversal {
    next_page: u32,
    task_id: String,
}

impl ReportPageTraversal {
    pub fn new(task_id: String) -> Result<Self, ReportError> {
        Ok(Self {
            next_page: 0,
            task_id: required_text(task_id, "report.task_id")?,
        })
    }

    #[must_use]
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    #[must_use]
    pub const fn next_page(&self) -> u32 {
        self.next_page
    }

    /// Provider pages and `pages_count` are zero-based: page 0 with pages_count 0 is complete.
    pub fn observe(&mut self, page: i32, pages_count: i32) -> Result<bool, ReportError> {
        let expected = i32::try_from(self.next_page).map_err(|_| ReportError::InvalidPage)?;
        if page != expected || pages_count < page {
            return Err(ReportError::InconsistentPagination {
                expected,
                actual: page,
                pages_count,
            });
        }
        if page == pages_count {
            Ok(true)
        } else {
            self.next_page = self
                .next_page
                .checked_add(1)
                .ok_or(ReportError::InvalidPage)?;
            Ok(false)
        }
    }
}

#[derive(Clone)]
pub struct ReportsClient {
    grpc: TInvestGrpcClient,
}

impl ReportsClient {
    pub fn new(grpc: TInvestGrpcClient) -> Self {
        Self { grpc }
    }

    pub async fn broker_report(
        &self,
        request: BrokerReportRequest,
    ) -> Result<BrokerReportState, ReportError> {
        let provider = match request {
            BrokerReportRequest::Generate {
                account_id,
                from,
                to,
            } => v1::BrokerReportRequest {
                payload: Some(
                    v1::broker_report_request::Payload::GenerateBrokerReportRequest(
                        v1::GenerateBrokerReportRequest {
                            account_id: required_text(account_id, "broker_report.account_id")?,
                            from: Some(timestamp(from)),
                            to: Some(timestamp(to)),
                        },
                    ),
                ),
            },
            BrokerReportRequest::Get { task_id, page } => v1::BrokerReportRequest {
                payload: Some(v1::broker_report_request::Payload::GetBrokerReportRequest(
                    v1::GetBrokerReportRequest {
                        task_id: required_text(task_id, "broker_report.task_id")?,
                        page: Some(page_i32(page)?),
                    },
                )),
            },
        };
        self.grpc
            .get_broker_report(provider)
            .await
            .map_err(ReportError::Provider)?
            .body
            .try_into()
    }

    pub async fn foreign_issuer_report(
        &self,
        request: ForeignIssuerReportRequest,
    ) -> Result<ForeignIssuerReportState, ReportError> {
        let provider = match request {
            ForeignIssuerReportRequest::Generate {
                account_id,
                from,
                to,
            } => {
                validate_same_calendar_year(from, to)?;
                v1::GetDividendsForeignIssuerRequest {
                    payload: Some(
                        v1::get_dividends_foreign_issuer_request::Payload::GenerateDivForeignIssuerReport(
                            v1::GenerateDividendsForeignIssuerReportRequest {
                                account_id: required_text(account_id, "foreign_report.account_id")?,
                                from: Some(timestamp(from)),
                                to: Some(timestamp(to)),
                            },
                        ),
                    ),
                }
            }
            ForeignIssuerReportRequest::Get { task_id, page } => {
                v1::GetDividendsForeignIssuerRequest {
                    payload: Some(
                        v1::get_dividends_foreign_issuer_request::Payload::GetDivForeignIssuerReport(
                            v1::GetDividendsForeignIssuerReportRequest {
                                task_id: required_text(task_id, "foreign_report.task_id")?,
                                page: Some(page_i32(page)?),
                            },
                        ),
                    ),
                }
            }
        };
        self.grpc
            .get_dividends_foreign_issuer(provider)
            .await
            .map_err(ReportError::Provider)?
            .body
            .try_into()
    }
}

impl TryFrom<v1::BrokerReportResponse> for BrokerReportState {
    type Error = ReportError;

    fn try_from(value: v1::BrokerReportResponse) -> Result<Self, Self::Error> {
        match value.payload {
            Some(v1::broker_report_response::Payload::GenerateBrokerReportResponse(response)) => {
                Ok(Self::Generating {
                    task_id: required_text(response.task_id, "broker_report.task_id")?,
                })
            }
            Some(v1::broker_report_response::Payload::GetBrokerReportResponse(response)) => {
                Ok(Self::Ready(response.try_into()?))
            }
            None => Err(ReportError::MissingPayload),
        }
    }
}

impl TryFrom<v1::GetBrokerReportResponse> for BrokerReportPage {
    type Error = ReportError;

    fn try_from(value: v1::GetBrokerReportResponse) -> Result<Self, Self::Error> {
        validate_page(value.page, value.pages_count)?;
        Ok(Self {
            task_id: optional_text(value.task_id),
            items_count: value.items_count,
            pages_count: value.pages_count,
            page: value.page,
            rows: value
                .broker_report
                .into_iter()
                .map(BrokerReportRow::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<v1::BrokerReport> for BrokerReportRow {
    type Error = ReportError;

    fn try_from(value: v1::BrokerReport) -> Result<Self, Self::Error> {
        Ok(Self {
            trade_id: optional_text(value.trade_id),
            order_id: optional_text(value.order_id),
            figi: optional_text(value.figi),
            execute_sign: optional_text(value.execute_sign),
            trade_at: optional_timestamp(value.trade_datetime)?,
            exchange: optional_text(value.exchange),
            class_code: optional_text(value.class_code),
            direction: optional_text(value.direction),
            name: optional_text(value.name),
            ticker: optional_text(value.ticker),
            price: optional_money(value.price)?,
            quantity: value.quantity,
            order_amount: optional_money(value.order_amount)?,
            accrued_interest: optional_quotation(value.aci_value)?,
            total_order_amount: optional_money(value.total_order_amount)?,
            broker_commission: optional_money(value.broker_commission)?,
            exchange_commission: optional_money(value.exchange_commission)?,
            clearing_commission: optional_money(value.exchange_clearing_commission)?,
            repo_rate: optional_quotation(value.repo_rate)?,
            party: optional_text(value.party),
            clear_value_at: optional_timestamp(value.clear_value_date)?,
            security_value_at: optional_timestamp(value.sec_value_date)?,
            broker_status: optional_text(value.broker_status),
            separate_agreement_type: optional_text(value.separate_agreement_type),
            separate_agreement_number: optional_text(value.separate_agreement_number),
            separate_agreement_date: optional_text(value.separate_agreement_date),
            delivery_type: optional_text(value.delivery_type),
        })
    }
}

impl TryFrom<v1::GetDividendsForeignIssuerResponse> for ForeignIssuerReportState {
    type Error = ReportError;

    fn try_from(value: v1::GetDividendsForeignIssuerResponse) -> Result<Self, Self::Error> {
        match value.payload {
            Some(v1::get_dividends_foreign_issuer_response::Payload::GenerateDivForeignIssuerReportResponse(response)) => {
                Ok(Self::Generating {
                    task_id: required_text(response.task_id, "foreign_report.task_id")?,
                })
            }
            Some(v1::get_dividends_foreign_issuer_response::Payload::DivForeignIssuerReport(response)) => {
                validate_page(response.page, response.pages_count)?;
                Ok(Self::Ready(ForeignIssuerReportPage {
                    items_count: response.items_count,
                    pages_count: response.pages_count,
                    page: response.page,
                    rows: response
                        .dividends_foreign_issuer_report
                        .into_iter()
                        .map(ForeignIssuerReportRow::try_from)
                        .collect::<Result<_, _>>()?,
                }))
            }
            None => Err(ReportError::MissingPayload),
        }
    }
}

impl TryFrom<v1::DividendsForeignIssuerReport> for ForeignIssuerReportRow {
    type Error = ReportError;

    fn try_from(value: v1::DividendsForeignIssuerReport) -> Result<Self, Self::Error> {
        Ok(Self {
            record_at: optional_timestamp(value.record_date)?,
            payment_at: optional_timestamp(value.payment_date)?,
            security_name: optional_text(value.security_name),
            isin: optional_text(value.isin),
            issuer_country: optional_text(value.issuer_country),
            quantity: value.quantity,
            dividend: optional_quotation(value.dividend)?,
            external_commission: optional_quotation(value.external_commission)?,
            dividend_gross: optional_quotation(value.dividend_gross)?,
            tax: optional_quotation(value.tax)?,
            dividend_amount: optional_quotation(value.dividend_amount)?,
            currency: optional_text(value.currency),
        })
    }
}

fn timestamp(value: ProviderTimestamp) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: value.seconds,
        nanos: value.nanos,
    }
}

fn page_i32(page: u32) -> Result<i32, ReportError> {
    i32::try_from(page).map_err(|_| ReportError::InvalidPage)
}

fn validate_page(page: i32, pages_count: i32) -> Result<(), ReportError> {
    if page < 0 || pages_count < 0 || (pages_count > 0 && page >= pages_count) {
        return Err(ReportError::InvalidPagination { page, pages_count });
    }
    Ok(())
}

fn validate_same_calendar_year(
    from: ProviderTimestamp,
    to: ProviderTimestamp,
) -> Result<(), ReportError> {
    if from > to {
        return Err(ReportError::InvalidRange);
    }
    // Civil year boundaries without floating point. `time` handles pre-epoch timestamps too.
    let from = time::OffsetDateTime::from_unix_timestamp(from.seconds)
        .map_err(|_| ReportError::InvalidRange)?;
    let to = time::OffsetDateTime::from_unix_timestamp(to.seconds)
        .map_err(|_| ReportError::InvalidRange)?;
    if from.year() != to.year() {
        return Err(ReportError::ForeignIssuerRangeCrossesCalendarYear);
    }
    Ok(())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReportError {
    #[error("{0}")]
    Provider(GrpcError),
    #[error("{0}")]
    Canonical(#[from] AccountDataError),
    #[error("provider omitted report response payload")]
    MissingPayload,
    #[error("report page does not fit provider int32 field")]
    InvalidPage,
    #[error("invalid report pagination page={page}, pages_count={pages_count}")]
    InvalidPagination { page: i32, pages_count: i32 },
    #[error("report start must not follow report end")]
    InvalidRange,
    #[error("foreign-issuer report range must stay within one calendar year")]
    ForeignIssuerRangeCrossesCalendarYear,
    #[error(
        "report pagination invalid: expected page {expected}, got {actual}, pages_count {pages_count}"
    )]
    InconsistentPagination {
        expected: i32,
        actual: i32,
        pages_count: i32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GrpcErrorKind, GrpcProviderError, GrpcRequestMetadata};
    use uuid::Uuid;

    #[test]
    fn report_generation_ready_empty_and_missing_variants_are_typed() {
        let generating = BrokerReportState::try_from(v1::BrokerReportResponse {
            payload: Some(
                v1::broker_report_response::Payload::GenerateBrokerReportResponse(
                    v1::GenerateBrokerReportResponse {
                        task_id: "task".to_owned(),
                    },
                ),
            ),
        })
        .expect("generation state");
        assert_eq!(
            generating,
            BrokerReportState::Generating {
                task_id: "task".to_owned()
            }
        );

        let ready = BrokerReportState::try_from(v1::BrokerReportResponse {
            payload: Some(
                v1::broker_report_response::Payload::GetBrokerReportResponse(
                    v1::GetBrokerReportResponse {
                        broker_report: Vec::new(),
                        items_count: 0,
                        pages_count: 0,
                        page: 0,
                        task_id: "task".to_owned(),
                    },
                ),
            ),
        })
        .expect("documented empty report");
        assert!(matches!(ready, BrokerReportState::Ready(page) if page.rows.is_empty()));
        assert_eq!(
            BrokerReportState::try_from(v1::BrokerReportResponse { payload: None }),
            Err(ReportError::MissingPayload)
        );
    }

    #[test]
    fn report_lifecycle_requires_task_readback_and_all_pages() {
        let mut traversal = ReportPageTraversal::new("task-1".into()).expect("task id");
        assert_eq!(traversal.task_id(), "task-1");
        assert_eq!(traversal.next_page(), 0);
        assert!(!traversal.observe(0, 2).expect("page 0"));
        assert!(!traversal.observe(1, 2).expect("page 1"));
        assert!(traversal.observe(2, 2).expect("final page"));

        let mut broken = ReportPageTraversal::new("task-2".into()).expect("task id");
        assert!(matches!(
            broken.observe(1, 2),
            Err(ReportError::InconsistentPagination { .. })
        ));
    }

    #[test]
    fn foreign_report_range_cannot_cross_year() {
        let from = ProviderTimestamp {
            seconds: 1_735_603_200,
            nanos: 0,
        };
        let to = ProviderTimestamp {
            seconds: 1_735_689_600,
            nanos: 0,
        };
        assert_eq!(
            validate_same_calendar_year(from, to),
            Err(ReportError::ForeignIssuerRangeCrossesCalendarYear)
        );
    }

    #[test]
    fn only_documented_capability_statuses_become_gates() {
        let error = |code| GrpcError {
            metadata: GrpcRequestMetadata {
                request_id: Uuid::nil(),
                method: "GetBrokerReport",
                attempt: 1,
                mutation: false,
            },
            kind: GrpcErrorKind::Provider(Box::new(GrpcProviderError {
                code,
                message: code.description().to_owned(),
                details: Vec::new(),
                tracking_id: None,
                rate_limit: Box::default(),
            })),
        };
        assert!(matches!(
            observe_documented_gate::<()>(
                "GetBrokerReport",
                Err(GrpcError {
                    kind: GrpcErrorKind::Provider(Box::new(GrpcProviderError {
                        code: tonic::Code::PermissionDenied,
                        message: "provider code 40002".to_owned(),
                        details: Vec::new(),
                        tracking_id: None,
                        rate_limit: Box::default(),
                    })),
                    ..error(tonic::Code::PermissionDenied)
                })
            ),
            Ok(CapabilityObservation::GatedUnavailable(
                CapabilityGate::Permission { .. }
            ))
        ));
        assert_eq!(
            observe_documented_gate::<()>(
                "GetBrokerReport",
                Err(error(tonic::Code::InvalidArgument))
            ),
            Err(error(tonic::Code::InvalidArgument))
        );
    }
}
