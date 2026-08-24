use serde::{Deserialize, Deserializer, Serialize};

macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdentityError::Empty);
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identity!(ClientRequestId);
identity!(ClientOrderId);
identity!(BrokerOrderId);
identity!(ExchangeOrderId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    Empty,
}

impl core::fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("identity cannot be empty")
    }
}

impl std::error::Error for IdentityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_remain_distinct_types_and_values() -> Result<(), IdentityError> {
        let client = ClientOrderId::new("same-text")?;
        let broker = BrokerOrderId::new("same-text")?;
        let exchange = ExchangeOrderId::new("same-text")?;
        assert_eq!(client.as_str(), broker.as_str());
        assert_eq!(broker.as_str(), exchange.as_str());
        Ok(())
    }

    #[test]
    fn empty_and_whitespace_only_identities_fail_closed() {
        assert_eq!(ClientRequestId::new(""), Err(IdentityError::Empty));
        assert_eq!(BrokerOrderId::new(" \t"), Err(IdentityError::Empty));
        assert!(serde_json::from_str::<ExchangeOrderId>("\"\"").is_err());
    }
}
