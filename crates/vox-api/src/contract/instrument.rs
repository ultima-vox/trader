//! Instrument identity on the public boundary.
//!
//! Vox already has a canonical provider-neutral instrument identity:
//! [`vox_domain::InstrumentIdentity`], documented in the domain as "provider-normalized
//! instrument identity retained beside runtime projections". #7 required that the UID, FIGI,
//! ticker and class code be retained *separately* rather than folded into one synthetic key,
//! and the accepted T-Invest layer keeps a typed `InstrumentUid` distinct from other uid
//! kinds. This module publishes that identity; it does not mint a second one.
//!
//! What the identity means:
//!
//! - `uid` is the provider's stable instrument identifier. It is meaningful **within a
//!   provider**, which is why the identity carries `provider` beside it. The pair is the
//!   identity; the uid alone is not.
//! - `figi`, `ticker` and `class_code` are aliases. They exist for display, search and
//!   cross-referencing, and none of them is the identity: tickers repeat across venues and
//!   are reused over time.
//! - Read models that carry a bare `instrument_uid` — positions, orders, stops, market data —
//!   are already inside a scope that names the provider, so the uid there is this same `uid`
//!   field, not a different identifier.
//!
//! A Vox-minted instrument id would be a second identity to keep in sync with the provider's
//! and would buy nothing today: there is one registered adapter, and the domain already
//! normalizes across providers by carrying the provider in the identity. If a second provider
//! ever lists the same economic instrument and Vox must state that the two are one thing,
//! that is the moment to introduce a Vox instrument id — and it belongs to the domain, not to
//! this transport.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use vox_domain::InstrumentIdentity;

/// The canonical instrument identity, as the domain defines it.
///
/// Identity is `provider` + `uid`. Everything else is an alias for humans and for lookup.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
pub struct InstrumentIdentityDto {
    /// Provider whose namespace `uid` belongs to.
    #[schema(example = "tinvest")]
    pub provider: String,
    /// The provider's stable instrument identifier. Diagnostics only in normal UI.
    #[schema(example = "e6123145-9665-43e0-8413-cd61b8aa9b13")]
    pub uid: String,
    /// FIGI where the provider supplies one. An alias, never the identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "BBG004730N88")]
    pub figi: Option<String>,
    /// Exchange ticker. Shown to operators; not unique across venues or over time.
    #[schema(example = "SBER")]
    pub ticker: String,
    /// Venue or board code that qualifies the ticker.
    #[schema(example = "TQBR")]
    pub class_code: String,
}

impl From<&InstrumentIdentity> for InstrumentIdentityDto {
    fn from(value: &InstrumentIdentity) -> Self {
        Self {
            provider: value.provider().to_owned(),
            uid: value.uid().to_owned(),
            figi: value.figi().map(ToOwned::to_owned),
            ticker: value.ticker().to_owned(),
            class_code: value.class_code().to_owned(),
        }
    }
}

impl InstrumentIdentityDto {
    /// The human label an operator reads: ticker and venue, never the uid.
    #[must_use]
    pub fn display(&self) -> String {
        format!("{} · {}", self.ticker, self.class_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> InstrumentIdentity {
        InstrumentIdentity::new(
            "tinvest",
            "e6123145-9665-43e0-8413-cd61b8aa9b13",
            Some("BBG004730N88".to_owned()),
            "SBER",
            "TQBR",
        )
        .expect("a complete identity")
    }

    #[test]
    fn the_public_type_is_the_domain_identity_not_a_new_one() {
        let dto = InstrumentIdentityDto::from(&identity());
        assert_eq!(dto.provider, "tinvest");
        assert_eq!(dto.uid, "e6123145-9665-43e0-8413-cd61b8aa9b13");
        assert_eq!(dto.figi.as_deref(), Some("BBG004730N88"));
        assert_eq!(dto.ticker, "SBER");
        assert_eq!(dto.class_code, "TQBR");
    }

    #[test]
    fn identity_carries_its_provider_because_a_uid_alone_is_not_one()
    -> Result<(), serde_json::Error> {
        let json = serde_json::to_value(InstrumentIdentityDto::from(&identity()))?;
        assert!(
            json.get("provider").is_some(),
            "a uid without its provider is not an identity"
        );
        assert!(
            json.get("instrument_id").is_none(),
            "no Vox-minted identifier exists: the provider identity is the identity"
        );
        Ok(())
    }

    #[test]
    fn operators_read_the_ticker_and_venue() {
        assert_eq!(
            InstrumentIdentityDto::from(&identity()).display(),
            "SBER · TQBR"
        );
    }
}
