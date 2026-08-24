# T-Invest reference-data ownership

Status: **ACTIVE**

`vox-tinvest::reference` owns T-Invest REST wire decoding and exposes typed Vox provider records. Raw JSON never leaves adapter. Provider UID, FIGI, ticker, class code, position UID and asset UID remain separate. Money, quotation, risk and analytics decimals have exact representations; adapter exposes no `f32`/`f64` financial API.

Provider catalogue remains complete even where Nautilus has no faithful instrument class. `vox-nautilus` alone may map validated shares, bonds, funds, currency pairs, futures and options into exact Nautilus runtime types. DFA, structured notes, indicatives, assets, issuer data, analytics, news and favorites remain Trader-owned reference data.

Missing trading-critical identity or economics fails mapping closed. Unknown provider enum strings remain explicit. Permission, environment, rollout and temporary failures update only affected method capability; they do not fabricate success or remove unrelated readiness.

Reference wire DTO optionality follows proto3 JSON presence, not business expectations. Any unset singular scalar, enum, timestamp, money, quotation, or nested message may be omitted by REST transcoding and is retained as `None`; repeated fields remain empty vectors. `Quotation` and `MoneyValue` also retain omitted `units`, `nano`, and `currency` components. Consumers must call explicit validators such as `try_identity`, `require_option_economics`, or `FuturesMargin::require_economics`; missing or invalid trading-critical data fails closed and is never converted to zero.

Current documented response fields have method-specific DTOs. Catalogue/asset records also retain future provider additions in `ProviderValue`: recursive Vox-owned values with exact decimal spelling, never `serde_json::Value` outside private decoding. This prevents silent field loss while keeping unknown schema evolution separate from known typed fields.
