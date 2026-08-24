# T-Invest reference-data ownership

Status: **ACTIVE**

`vox-tinvest::reference` owns T-Invest REST wire decoding and exposes typed Vox provider records. Raw JSON never leaves adapter. Provider UID, FIGI, ticker, class code, position UID and asset UID remain separate. Money, quotation, risk and analytics decimals have exact representations; adapter exposes no `f32`/`f64` financial API.

Provider catalogue remains complete even where Nautilus has no faithful instrument class. `vox-nautilus` alone may map validated shares, bonds, funds, currency pairs, futures and options into exact Nautilus runtime types. DFA, structured notes, indicatives, assets, issuer data, analytics, news and favorites remain Trader-owned reference data.

Missing trading-critical identity or economics fails mapping closed. Unknown provider enum strings remain explicit. Permission, environment, rollout and temporary failures update only affected method capability; they do not fabricate success or remove unrelated readiness.
