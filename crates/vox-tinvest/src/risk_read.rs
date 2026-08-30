//! Broker-first read adapter for #21 risk.
//!
//! This adapter reuses the already resolved T-Invest client from #17. It does not own
//! policy and does not derive provider truth.

use thiserror::Error;
use vox_connections::BrokerEnvironment;
use vox_domain::{FixedPoint, OrderSide, UnitsNano};

use crate::canonical::CanonicalMoney;
use crate::execution::{CanonicalMaxLots, CanonicalOrderPrice, ExecutionDecodeError};
use crate::generated::v1;
use crate::{GrpcError, TInvestGrpcClient};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalMarginAttributes {
    pub liquid_portfolio: Option<CanonicalMoney>,
    pub starting_margin: Option<CanonicalMoney>,
    pub minimal_margin: Option<CanonicalMoney>,
    pub funds_sufficiency_level: Option<UnitsNano>,
    pub amount_of_missing_funds: Option<CanonicalMoney>,
    pub corrected_margin: Option<CanonicalMoney>,
    pub guarantee_for_futures: Option<CanonicalMoney>,
}

impl TryFrom<v1::GetMarginAttributesResponse> for CanonicalMarginAttributes {
    type Error = RiskReadError;

    fn try_from(value: v1::GetMarginAttributesResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            liquid_portfolio: value.liquid_portfolio.map(TryInto::try_into).transpose()?,
            starting_margin: value.starting_margin.map(TryInto::try_into).transpose()?,
            minimal_margin: value.minimal_margin.map(TryInto::try_into).transpose()?,
            funds_sufficiency_level: value
                .funds_sufficiency_level
                .map(|value| UnitsNano::new(value.units, value.nano))
                .transpose()
                .map_err(|_| RiskReadError::InvalidEconomics)?,
            amount_of_missing_funds: value
                .amount_of_missing_funds
                .map(TryInto::try_into)
                .transpose()?,
            corrected_margin: value.corrected_margin.map(TryInto::try_into).transpose()?,
            guarantee_for_futures: value
                .guarantee_for_futures
                .map(TryInto::try_into)
                .transpose()?,
        })
    }
}

#[derive(Clone)]
pub struct TInvestRiskReadAdapter {
    client: TInvestGrpcClient,
    environment: BrokerEnvironment,
}

impl TInvestRiskReadAdapter {
    #[must_use]
    pub const fn new(client: TInvestGrpcClient, environment: BrokerEnvironment) -> Self {
        Self { client, environment }
    }

    pub async fn margin_attributes(
        &self,
        account_id: &str,
    ) -> Result<CanonicalMarginAttributes, RiskReadError> {
        let response = self
            .client
            .get_margin_attributes(v1::GetMarginAttributesRequest {
                account_id: account_id.to_owned(),
            })
            .await?
            .body;
        response.try_into()
    }

    pub async fn max_lots(
        &self,
        account_id: &str,
        instrument_id: &str,
        price: FixedPoint,
    ) -> Result<CanonicalMaxLots, RiskReadError> {
        let request = v1::GetMaxLotsRequest {
            account_id: account_id.to_owned(),
            instrument_id: instrument_id.to_owned(),
            price: Some(quotation(price)?),
        };
        let response = match self.environment {
            BrokerEnvironment::Sandbox => self.client.get_sandbox_max_lots(request).await?.body,
            BrokerEnvironment::Production => self.client.get_max_lots(request).await?.body,
        };
        response.try_into().map_err(Into::into)
    }

    /// T-Invest documents GetOrderPrice as a preliminary estimate for a limit order.
    /// This method is deliberately limit-specific so callers cannot use it as a market-price oracle.
    pub async fn limit_order_price(
        &self,
        account_id: &str,
        instrument_id: &str,
        price: FixedPoint,
        side: OrderSide,
        quantity_lots: i64,
    ) -> Result<CanonicalOrderPrice, RiskReadError> {
        if quantity_lots <= 0 {
            return Err(RiskReadError::InvalidQuantity);
        }
        let direction = match side {
            OrderSide::Buy => v1::OrderDirection::Buy as i32,
            OrderSide::Sell => v1::OrderDirection::Sell as i32,
        };
        let request = v1::GetOrderPriceRequest {
            account_id: account_id.to_owned(),
            instrument_id: instrument_id.to_owned(),
            price: Some(quotation(price)?),
            direction,
            quantity: quantity_lots,
        };
        let response = match self.environment {
            BrokerEnvironment::Sandbox => self.client.get_sandbox_order_price(request).await?.body,
            BrokerEnvironment::Production => self.client.get_order_price(request).await?.body,
        };
        response.try_into().map_err(Into::into)
    }
}

fn quotation(value: FixedPoint) -> Result<v1::Quotation, RiskReadError> {
    let (units, nano) = value.units_nano();
    Ok(v1::Quotation {
        units: i64::try_from(units).map_err(|_| RiskReadError::InvalidEconomics)?,
        nano,
    })
}

#[derive(Debug, Error)]
pub enum RiskReadError {
    #[error("T-Invest risk read failed: {0}")]
    Grpc(#[from] GrpcError),
    #[error("T-Invest execution fact decode failed: {0}")]
    ExecutionDecode(#[from] ExecutionDecodeError),
    #[error("T-Invest economics decode failed")]
    Economics(#[from] crate::canonical::EconomicsError),
    #[error("risk read economics are not exactly representable")]
    InvalidEconomics,
    #[error("risk read quantity must be positive")]
    InvalidQuantity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_attributes_preserve_corrected_margin_as_provider_fact() {
        let response = v1::GetMarginAttributesResponse {
            corrected_margin: Some(v1::MoneyValue {
                currency: "rub".to_owned(),
                units: 12,
                nano: 340_000_000,
            }),
            funds_sufficiency_level: Some(v1::Quotation {
                units: 2,
                nano: 0,
            }),
            ..Default::default()
        };
        let canonical = CanonicalMarginAttributes::try_from(response).expect("canonical margin");
        assert_eq!(
            canonical
                .corrected_margin
                .expect("corrected margin")
                .amount
                .fixed_point()
                .total_nanos(),
            12_340_000_000
        );
        assert_eq!(
            canonical
                .funds_sufficiency_level
                .expect("sufficiency")
                .fixed_point()
                .total_nanos(),
            2_000_000_000
        );
    }

    #[test]
    fn exact_fixed_point_becomes_exact_provider_quotation() {
        let q = quotation(FixedPoint::from_total_nanos(-1_250_000_000)).expect("quotation");
        assert_eq!(q.units, -1);
        assert_eq!(q.nano, -250_000_000);
    }
}
