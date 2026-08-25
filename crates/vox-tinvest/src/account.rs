//! Vox-owned account and portfolio read models.
//!
//! Generated protobuf messages stop at this module boundary. Every optional provider
//! economic remains optional and every enum keeps its original wire number.

use prost_types::Timestamp;
use thiserror::Error;
use vox_domain::UnitsNano;

use crate::canonical::CanonicalMoney;
use crate::generated::v1;
use crate::{GrpcError, TInvestGrpcClient};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ProviderTimestamp {
    pub seconds: i64,
    pub nanos: i32,
}

impl TryFrom<Timestamp> for ProviderTimestamp {
    type Error = AccountDataError;

    fn try_from(value: Timestamp) -> Result<Self, Self::Error> {
        if !(0..1_000_000_000).contains(&value.nanos) {
            return Err(AccountDataError::InvalidTimestamp);
        }
        Ok(Self {
            seconds: value.seconds,
            nanos: value.nanos,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountCatalogue {
    pub accounts: Vec<CanonicalAccount>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalAccount {
    pub account_id: String,
    pub account_type: i32,
    pub name: Option<String>,
    pub status: i32,
    pub opened_at: Option<ProviderTimestamp>,
    pub closed_at: Option<ProviderTimestamp>,
    pub access_level: i32,
}

impl TryFrom<v1::GetAccountsResponse> for AccountCatalogue {
    type Error = AccountDataError;

    fn try_from(value: v1::GetAccountsResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            accounts: value
                .accounts
                .into_iter()
                .map(CanonicalAccount::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<v1::Account> for CanonicalAccount {
    type Error = AccountDataError;

    fn try_from(value: v1::Account) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: required_text(value.id, "account.id")?,
            account_type: value.r#type,
            name: optional_text(value.name),
            status: value.status,
            opened_at: optional_timestamp(value.opened_date)?,
            closed_at: optional_timestamp(value.closed_date)?,
            access_level: value.access_level,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarginAttributes {
    pub liquid_portfolio: Option<CanonicalMoney>,
    pub starting_margin: Option<CanonicalMoney>,
    pub minimal_margin: Option<CanonicalMoney>,
    pub funds_sufficiency_level: Option<UnitsNano>,
    pub amount_of_missing_funds: Option<CanonicalMoney>,
    pub corrected_margin: Option<CanonicalMoney>,
    pub guarantee_for_futures: Option<CanonicalMoney>,
}

impl TryFrom<v1::GetMarginAttributesResponse> for MarginAttributes {
    type Error = AccountDataError;

    fn try_from(value: v1::GetMarginAttributesResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            liquid_portfolio: optional_money(value.liquid_portfolio)?,
            starting_margin: optional_money(value.starting_margin)?,
            minimal_margin: optional_money(value.minimal_margin)?,
            funds_sufficiency_level: optional_quotation(value.funds_sufficiency_level)?,
            amount_of_missing_funds: optional_money(value.amount_of_missing_funds)?,
            corrected_margin: optional_money(value.corrected_margin)?,
            guarantee_for_futures: optional_money(value.guarantee_for_futures)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserTariff {
    pub unary_limits: Vec<UnaryLimit>,
    pub stream_limits: Vec<StreamLimit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnaryLimit {
    pub limit_per_minute: i32,
    pub methods: Vec<String>,
    pub limit_per_second: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamLimit {
    pub limit: i32,
    pub streams: Vec<String>,
    pub open: i32,
}

impl From<v1::GetUserTariffResponse> for UserTariff {
    fn from(value: v1::GetUserTariffResponse) -> Self {
        Self {
            unary_limits: value
                .unary_limits
                .into_iter()
                .map(|limit| UnaryLimit {
                    limit_per_minute: limit.limit_per_minute,
                    methods: limit.methods,
                    limit_per_second: limit.limit_per_second,
                })
                .collect(),
            stream_limits: value
                .stream_limits
                .into_iter()
                .map(|limit| StreamLimit {
                    limit: limit.limit,
                    streams: limit.streams,
                    open: limit.open,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserInfo {
    pub premium: bool,
    pub qualified: bool,
    pub qualified_for: Vec<String>,
    pub tariff: Option<String>,
    pub user_id: Option<String>,
    pub risk_level_code: Option<String>,
}

impl From<v1::GetInfoResponse> for UserInfo {
    fn from(value: v1::GetInfoResponse) -> Self {
        Self {
            premium: value.prem_status,
            qualified: value.qual_status,
            qualified_for: value.qualified_for_work_with,
            tariff: optional_text(value.tariff),
            user_id: optional_text(value.user_id),
            risk_level_code: optional_text(value.risk_level_code),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BankAccount {
    pub account_id: String,
    pub name: Option<String>,
    pub money: Vec<CanonicalMoney>,
    pub opened_at: Option<ProviderTimestamp>,
    pub account_type: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BankAccountCatalogue {
    pub accounts: Vec<BankAccount>,
}

impl TryFrom<v1::GetBankAccountsResponse> for BankAccountCatalogue {
    type Error = AccountDataError;

    fn try_from(value: v1::GetBankAccountsResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            accounts: value
                .bank_accounts
                .into_iter()
                .map(BankAccount::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<v1::BankAccount> for BankAccount {
    type Error = AccountDataError;

    fn try_from(value: v1::BankAccount) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: required_text(value.id, "bank_account.id")?,
            name: optional_text(value.name),
            money: money_values(value.money)?,
            opened_at: optional_timestamp(value.opened_date)?,
            account_type: value.r#type,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountValues {
    pub accounts: Vec<AccountValueSet>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountValuesQuery {
    pub account_ids: Vec<String>,
    pub values: Vec<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioQuery {
    pub account_id: String,
    pub currency: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountValueSet {
    pub account_id: String,
    pub values: Vec<AccountValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountValue {
    pub name: i32,
    pub value: Option<CanonicalMoney>,
}

impl TryFrom<v1::GetAccountValuesResponse> for AccountValues {
    type Error = AccountDataError;

    fn try_from(value: v1::GetAccountValuesResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            accounts: value
                .accounts
                .into_iter()
                .map(|account| {
                    Ok(AccountValueSet {
                        account_id: required_text(account.account_id, "account_values.account_id")?,
                        values: account
                            .values
                            .into_iter()
                            .map(|parameter| {
                                Ok(AccountValue {
                                    name: parameter.name,
                                    value: optional_money(parameter.value)?,
                                })
                            })
                            .collect::<Result<_, AccountDataError>>()?,
                    })
                })
                .collect::<Result<_, AccountDataError>>()?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionsState {
    pub account_id: Option<String>,
    pub money: Vec<CanonicalMoney>,
    pub blocked_money: Vec<CanonicalMoney>,
    pub securities: Vec<SecurityPosition>,
    pub futures: Vec<FuturePosition>,
    pub options: Vec<OptionPosition>,
    pub limits_loading_in_progress: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionIdentity {
    pub instrument_uid: Option<String>,
    pub position_uid: Option<String>,
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityPosition {
    pub identity: PositionIdentity,
    pub instrument_type: Option<String>,
    pub blocked: i64,
    pub balance: i64,
    pub exchange_blocked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FuturePosition {
    pub identity: PositionIdentity,
    pub blocked: i64,
    pub balance: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionPosition {
    pub identity: PositionIdentity,
    pub blocked: i64,
    pub balance: i64,
}

impl TryFrom<v1::PositionsResponse> for PositionsState {
    type Error = AccountDataError;

    fn try_from(value: v1::PositionsResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: optional_text(value.account_id),
            money: money_values(value.money)?,
            blocked_money: money_values(value.blocked)?,
            securities: value
                .securities
                .into_iter()
                .map(SecurityPosition::from)
                .collect(),
            futures: value
                .futures
                .into_iter()
                .map(FuturePosition::from)
                .collect(),
            options: value
                .options
                .into_iter()
                .map(OptionPosition::from)
                .collect(),
            limits_loading_in_progress: value.limits_loading_in_progress,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioState {
    pub account_id: Option<String>,
    pub total_amount_shares: Option<CanonicalMoney>,
    pub total_amount_bonds: Option<CanonicalMoney>,
    pub total_amount_etf: Option<CanonicalMoney>,
    pub total_amount_currencies: Option<CanonicalMoney>,
    pub total_amount_futures: Option<CanonicalMoney>,
    pub total_amount_options: Option<CanonicalMoney>,
    pub total_amount_structured_products: Option<CanonicalMoney>,
    pub total_amount_portfolio: Option<CanonicalMoney>,
    pub total_amount_dfa: Option<CanonicalMoney>,
    pub expected_yield: Option<UnitsNano>,
    pub daily_yield: Option<CanonicalMoney>,
    pub daily_yield_relative: Option<UnitsNano>,
    pub positions: Vec<PortfolioPosition>,
    pub virtual_positions: Vec<VirtualPortfolioPosition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioPosition {
    pub identity: PositionIdentity,
    pub instrument_type: Option<String>,
    pub quantity: Option<UnitsNano>,
    pub average_position_price: Option<CanonicalMoney>,
    pub expected_yield: Option<UnitsNano>,
    pub current_nkd: Option<CanonicalMoney>,
    pub average_position_price_points: Option<UnitsNano>,
    pub current_price: Option<CanonicalMoney>,
    pub average_position_price_fifo: Option<CanonicalMoney>,
    pub quantity_lots: Option<UnitsNano>,
    pub blocked: bool,
    pub blocked_lots: Option<UnitsNano>,
    pub var_margin: Option<CanonicalMoney>,
    pub var_margin_settled: Option<CanonicalMoney>,
    pub expected_yield_fifo: Option<UnitsNano>,
    pub daily_yield: Option<CanonicalMoney>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualPortfolioPosition {
    pub identity: PositionIdentity,
    pub instrument_type: Option<String>,
    pub quantity: Option<UnitsNano>,
    pub average_position_price: Option<CanonicalMoney>,
    pub expected_yield: Option<UnitsNano>,
    pub expected_yield_fifo: Option<UnitsNano>,
    pub expires_at: Option<ProviderTimestamp>,
    pub current_price: Option<CanonicalMoney>,
    pub average_position_price_fifo: Option<CanonicalMoney>,
    pub daily_yield: Option<CanonicalMoney>,
}

impl TryFrom<v1::PortfolioResponse> for PortfolioState {
    type Error = AccountDataError;

    fn try_from(value: v1::PortfolioResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: optional_text(value.account_id),
            total_amount_shares: optional_money(value.total_amount_shares)?,
            total_amount_bonds: optional_money(value.total_amount_bonds)?,
            total_amount_etf: optional_money(value.total_amount_etf)?,
            total_amount_currencies: optional_money(value.total_amount_currencies)?,
            total_amount_futures: optional_money(value.total_amount_futures)?,
            total_amount_options: optional_money(value.total_amount_options)?,
            total_amount_structured_products: optional_money(value.total_amount_sp)?,
            total_amount_portfolio: optional_money(value.total_amount_portfolio)?,
            total_amount_dfa: optional_money(value.total_amount_dfa)?,
            expected_yield: optional_quotation(value.expected_yield)?,
            daily_yield: optional_money(value.daily_yield)?,
            daily_yield_relative: optional_quotation(value.daily_yield_relative)?,
            positions: value
                .positions
                .into_iter()
                .map(PortfolioPosition::try_from)
                .collect::<Result<_, _>>()?,
            virtual_positions: value
                .virtual_positions
                .into_iter()
                .map(VirtualPortfolioPosition::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[allow(deprecated)]
impl TryFrom<v1::PortfolioPosition> for PortfolioPosition {
    type Error = AccountDataError;

    fn try_from(value: v1::PortfolioPosition) -> Result<Self, Self::Error> {
        Ok(Self {
            identity: position_identity(
                value.instrument_uid,
                value.position_uid,
                value.figi,
                value.ticker,
                value.class_code,
            ),
            instrument_type: optional_text(value.instrument_type),
            quantity: optional_quotation(value.quantity)?,
            average_position_price: optional_money(value.average_position_price)?,
            expected_yield: optional_quotation(value.expected_yield)?,
            current_nkd: optional_money(value.current_nkd)?,
            average_position_price_points: optional_quotation(value.average_position_price_pt)?,
            current_price: optional_money(value.current_price)?,
            average_position_price_fifo: optional_money(value.average_position_price_fifo)?,
            quantity_lots: optional_quotation(value.quantity_lots)?,
            blocked: value.blocked,
            blocked_lots: optional_quotation(value.blocked_lots)?,
            var_margin: optional_money(value.var_margin)?,
            var_margin_settled: optional_money(value.var_margin_settled)?,
            expected_yield_fifo: optional_quotation(value.expected_yield_fifo)?,
            daily_yield: optional_money(value.daily_yield)?,
        })
    }
}

impl TryFrom<v1::VirtualPortfolioPosition> for VirtualPortfolioPosition {
    type Error = AccountDataError;

    fn try_from(value: v1::VirtualPortfolioPosition) -> Result<Self, Self::Error> {
        Ok(Self {
            identity: position_identity(
                value.instrument_uid,
                value.position_uid,
                value.figi,
                value.ticker,
                value.class_code,
            ),
            instrument_type: optional_text(value.instrument_type),
            quantity: optional_quotation(value.quantity)?,
            average_position_price: optional_money(value.average_position_price)?,
            expected_yield: optional_quotation(value.expected_yield)?,
            expected_yield_fifo: optional_quotation(value.expected_yield_fifo)?,
            expires_at: optional_timestamp(value.expire_date)?,
            current_price: optional_money(value.current_price)?,
            average_position_price_fifo: optional_money(value.average_position_price_fifo)?,
            daily_yield: optional_money(value.daily_yield)?,
        })
    }
}

impl From<v1::PositionsSecurities> for SecurityPosition {
    fn from(value: v1::PositionsSecurities) -> Self {
        Self {
            identity: position_identity(
                value.instrument_uid,
                value.position_uid,
                value.figi,
                value.ticker,
                value.class_code,
            ),
            instrument_type: optional_text(value.instrument_type),
            blocked: value.blocked,
            balance: value.balance,
            exchange_blocked: value.exchange_blocked,
        }
    }
}

impl From<v1::PositionsFutures> for FuturePosition {
    fn from(value: v1::PositionsFutures) -> Self {
        Self {
            identity: position_identity(
                value.instrument_uid,
                value.position_uid,
                value.figi,
                value.ticker,
                value.class_code,
            ),
            blocked: value.blocked,
            balance: value.balance,
        }
    }
}

impl From<v1::PositionsOptions> for OptionPosition {
    fn from(value: v1::PositionsOptions) -> Self {
        Self {
            identity: position_identity(
                value.instrument_uid,
                value.position_uid,
                String::new(),
                value.ticker,
                value.class_code,
            ),
            blocked: value.blocked,
            balance: value.balance,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawLimits {
    pub money: Vec<CanonicalMoney>,
    pub blocked: Vec<CanonicalMoney>,
    pub blocked_guarantee: Vec<CanonicalMoney>,
}

impl TryFrom<v1::WithdrawLimitsResponse> for WithdrawLimits {
    type Error = AccountDataError;

    fn try_from(value: v1::WithdrawLimitsResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            money: money_values(value.money)?,
            blocked: money_values(value.blocked)?,
            blocked_guarantee: money_values(value.blocked_guarantee)?,
        })
    }
}

fn position_identity(
    instrument_uid: String,
    position_uid: String,
    figi: String,
    ticker: String,
    class_code: String,
) -> PositionIdentity {
    PositionIdentity {
        instrument_uid: optional_text(instrument_uid),
        position_uid: optional_text(position_uid),
        figi: optional_text(figi),
        ticker: optional_text(ticker),
        class_code: optional_text(class_code),
    }
}

pub(crate) fn optional_text(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

pub(crate) fn required_text(
    value: String,
    field: &'static str,
) -> Result<String, AccountDataError> {
    optional_text(value).ok_or(AccountDataError::MissingIdentity(field))
}

pub(crate) fn optional_timestamp(
    value: Option<Timestamp>,
) -> Result<Option<ProviderTimestamp>, AccountDataError> {
    value.map(TryInto::try_into).transpose()
}

pub(crate) fn optional_money(
    value: Option<v1::MoneyValue>,
) -> Result<Option<CanonicalMoney>, AccountDataError> {
    value
        .map(CanonicalMoney::try_from)
        .transpose()
        .map_err(|_| AccountDataError::InvalidUnitsNano)
}

pub(crate) fn optional_quotation(
    value: Option<v1::Quotation>,
) -> Result<Option<UnitsNano>, AccountDataError> {
    value
        .map(|value| {
            UnitsNano::new(value.units, value.nano).map_err(|_| AccountDataError::InvalidUnitsNano)
        })
        .transpose()
}

pub(crate) fn money_values(
    values: Vec<v1::MoneyValue>,
) -> Result<Vec<CanonicalMoney>, AccountDataError> {
    values
        .into_iter()
        .map(|value| {
            CanonicalMoney::try_from(value).map_err(|_| AccountDataError::InvalidUnitsNano)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AccountDataError {
    #[error("missing required provider identity: {0}")]
    MissingIdentity(&'static str),
    #[error("provider units/nano value is outside canonical range")]
    InvalidUnitsNano,
    #[error("provider timestamp nanos are outside 0..1,000,000,000")]
    InvalidTimestamp,
    #[error("provider response identity differs from request: {0}")]
    IdentityMismatch(&'static str),
}

#[derive(Clone)]
pub struct AccountReadClient {
    grpc: TInvestGrpcClient,
}

impl AccountReadClient {
    pub fn new(grpc: TInvestGrpcClient) -> Self {
        Self { grpc }
    }

    pub async fn accounts(
        &self,
        status: Option<i32>,
    ) -> Result<AccountCatalogue, AccountReadError> {
        self.grpc
            .get_accounts(v1::GetAccountsRequest { status })
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }

    pub async fn margin_attributes(
        &self,
        account_id: String,
    ) -> Result<MarginAttributes, AccountReadError> {
        self.grpc
            .get_margin_attributes(v1::GetMarginAttributesRequest { account_id })
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }

    pub async fn tariff(&self) -> Result<UserTariff, AccountReadError> {
        Ok(self
            .grpc
            .get_user_tariff(v1::GetUserTariffRequest {})
            .await?
            .body
            .into())
    }

    pub async fn user_info(&self) -> Result<UserInfo, AccountReadError> {
        Ok(self.grpc.get_info(v1::GetInfoRequest {}).await?.body.into())
    }

    pub async fn bank_accounts(&self) -> Result<BankAccountCatalogue, AccountReadError> {
        self.grpc
            .get_bank_accounts(v1::GetBankAccountsRequest {})
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }

    pub async fn account_values(
        &self,
        query: AccountValuesQuery,
    ) -> Result<AccountValues, AccountReadError> {
        self.grpc
            .get_account_values(v1::GetAccountValuesRequest {
                accounts: query.account_ids,
                values: query.values,
            })
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }

    pub async fn portfolio(
        &self,
        query: PortfolioQuery,
    ) -> Result<PortfolioState, AccountReadError> {
        let expected = query.account_id.clone();
        let state: PortfolioState = self
            .grpc
            .get_portfolio(v1::PortfolioRequest {
                account_id: query.account_id,
                currency: query.currency,
            })
            .await?
            .body
            .try_into()
            .map_err(AccountReadError::from)?;
        validate_response_account(&expected, state.account_id.as_deref())?;
        Ok(state)
    }

    pub async fn positions(&self, account_id: String) -> Result<PositionsState, AccountReadError> {
        let expected = account_id.clone();
        let state: PositionsState = self
            .grpc
            .get_positions(v1::PositionsRequest { account_id })
            .await?
            .body
            .try_into()
            .map_err(AccountReadError::from)?;
        validate_response_account(&expected, state.account_id.as_deref())?;
        Ok(state)
    }

    pub async fn withdraw_limits(
        &self,
        account_id: String,
    ) -> Result<WithdrawLimits, AccountReadError> {
        self.grpc
            .get_withdraw_limits(v1::WithdrawLimitsRequest { account_id })
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }

    pub async fn sandbox_accounts(&self) -> Result<AccountCatalogue, AccountReadError> {
        self.grpc
            .get_sandbox_accounts(v1::GetAccountsRequest::default())
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }

    pub async fn sandbox_portfolio(
        &self,
        account_id: String,
    ) -> Result<PortfolioState, AccountReadError> {
        let expected = account_id.clone();
        let state: PortfolioState = self
            .grpc
            .get_sandbox_portfolio(v1::PortfolioRequest {
                account_id,
                currency: None,
            })
            .await?
            .body
            .try_into()
            .map_err(AccountReadError::from)?;
        validate_response_account(&expected, state.account_id.as_deref())?;
        Ok(state)
    }

    pub async fn sandbox_positions(
        &self,
        account_id: String,
    ) -> Result<PositionsState, AccountReadError> {
        let expected = account_id.clone();
        let state: PositionsState = self
            .grpc
            .get_sandbox_positions(v1::PositionsRequest { account_id })
            .await?
            .body
            .try_into()
            .map_err(AccountReadError::from)?;
        validate_response_account(&expected, state.account_id.as_deref())?;
        Ok(state)
    }

    pub async fn sandbox_withdraw_limits(
        &self,
        account_id: String,
    ) -> Result<WithdrawLimits, AccountReadError> {
        self.grpc
            .get_sandbox_withdraw_limits(v1::WithdrawLimitsRequest { account_id })
            .await?
            .body
            .try_into()
            .map_err(Into::into)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AccountReadError {
    #[error("{0}")]
    Provider(#[from] GrpcError),
    #[error("{0}")]
    Canonical(#[from] AccountDataError),
    #[error("provider response omitted account identity for request {expected}")]
    MissingResponseAccount { expected: String },
    #[error("provider response account {actual} differs from request {expected}")]
    ResponseAccountMismatch { expected: String, actual: String },
}

fn validate_response_account(expected: &str, actual: Option<&str>) -> Result<(), AccountReadError> {
    match actual {
        None => Err(AccountReadError::MissingResponseAccount {
            expected: expected.to_owned(),
        }),
        Some(actual) if actual != expected => Err(AccountReadError::ResponseAccountMismatch {
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        }),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_omissions_and_unknown_account_enums_survive() {
        let margin = MarginAttributes::try_from(v1::GetMarginAttributesResponse {
            liquid_portfolio: None,
            starting_margin: None,
            minimal_margin: None,
            funds_sufficiency_level: None,
            amount_of_missing_funds: None,
            corrected_margin: None,
            guarantee_for_futures: None,
        })
        .expect("empty optional economics are valid");
        assert_eq!(margin.liquid_portfolio, None);

        let account = CanonicalAccount::try_from(v1::Account {
            id: "broker-account".to_owned(),
            r#type: 99_001,
            name: String::new(),
            status: 99_002,
            opened_date: None,
            closed_date: None,
            access_level: 99_003,
        })
        .expect("valid identity");
        assert_eq!(
            (account.account_type, account.status, account.access_level),
            (99_001, 99_002, 99_003)
        );
    }

    #[test]
    fn multiple_currency_and_empty_positions_are_valid() {
        let positions = PositionsState::try_from(v1::PositionsResponse {
            money: vec![
                v1::MoneyValue {
                    currency: "rub".to_owned(),
                    units: 1,
                    nano: 0,
                },
                v1::MoneyValue {
                    currency: "usd".to_owned(),
                    units: 2,
                    nano: 1,
                },
            ],
            blocked: Vec::new(),
            securities: Vec::new(),
            limits_loading_in_progress: false,
            futures: Vec::new(),
            options: Vec::new(),
            account_id: "account".to_owned(),
        })
        .expect("valid positions");
        assert_eq!(positions.money.len(), 2);
        assert!(positions.securities.is_empty());
    }

    #[test]
    fn all_position_families_and_provider_subidentities_survive() {
        let positions = PositionsState::try_from(v1::PositionsResponse {
            securities: vec![v1::PositionsSecurities {
                figi: "figi-share".to_owned(),
                blocked: 1,
                balance: 2,
                position_uid: "position-share".to_owned(),
                instrument_uid: "instrument-share".to_owned(),
                ticker: "SHARE".to_owned(),
                class_code: "TQBR".to_owned(),
                exchange_blocked: true,
                instrument_type: "share".to_owned(),
            }],
            futures: vec![v1::PositionsFutures {
                figi: "figi-future".to_owned(),
                blocked: 3,
                balance: 4,
                position_uid: "position-future".to_owned(),
                instrument_uid: "instrument-future".to_owned(),
                ticker: "FUT".to_owned(),
                class_code: "SPBFUT".to_owned(),
            }],
            options: vec![v1::PositionsOptions {
                position_uid: "position-option".to_owned(),
                instrument_uid: "instrument-option".to_owned(),
                ticker: "OPT".to_owned(),
                class_code: "SPBOPT".to_owned(),
                blocked: 5,
                balance: 6,
            }],
            ..Default::default()
        })
        .expect("all current position shapes");
        assert_eq!(
            positions.securities[0].identity.figi.as_deref(),
            Some("figi-share")
        );
        assert_eq!(
            positions.futures[0].identity.instrument_uid.as_deref(),
            Some("instrument-future")
        );
        assert_eq!(
            positions.options[0].identity.position_uid.as_deref(),
            Some("position-option")
        );
    }

    #[test]
    fn populated_portfolio_and_account_value_unknown_variant_remain_exact() {
        let portfolio = PortfolioState::try_from(v1::PortfolioResponse {
            total_amount_portfolio: Some(v1::MoneyValue {
                currency: "rub".to_owned(),
                units: 42,
                nano: 123,
            }),
            expected_yield: Some(v1::Quotation {
                units: -1,
                nano: -1,
            }),
            positions: vec![v1::PortfolioPosition {
                instrument_uid: "instrument".to_owned(),
                position_uid: "position".to_owned(),
                figi: "figi".to_owned(),
                quantity: Some(v1::Quotation { units: 2, nano: 0 }),
                ..Default::default()
            }],
            account_id: "account".to_owned(),
            ..Default::default()
        })
        .expect("populated portfolio");
        assert_eq!(portfolio.positions.len(), 1);
        assert!(portfolio.total_amount_portfolio.is_some());

        let values = AccountValues::try_from(v1::GetAccountValuesResponse {
            accounts: vec![v1::AccountValuesWithParameters {
                account_id: "account".to_owned(),
                values: vec![v1::InstrumentParameter {
                    name: 99_999,
                    value: None,
                }],
            }],
        })
        .expect("future account value enum");
        assert_eq!(values.accounts[0].values[0].name, 99_999);
        assert_eq!(values.accounts[0].values[0].value, None);
    }

    #[test]
    fn account_scoped_response_identity_must_match_request() {
        assert_eq!(
            validate_response_account("expected", Some("expected")),
            Ok(())
        );
        assert!(matches!(
            validate_response_account("expected", None),
            Err(AccountReadError::MissingResponseAccount { .. })
        ));
        assert!(matches!(
            validate_response_account("expected", Some("other")),
            Err(AccountReadError::ResponseAccountMismatch { .. })
        ));
    }
}
