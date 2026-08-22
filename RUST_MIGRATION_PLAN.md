# Vox Trader — Rust migration gate

Status: **ACTIVE**

This plan implements ADR-0002 before feature development continues.

## Objective

Replace the transitional Python production seed with a Rust-first Vox Core foundation while preserving all qualification evidence and safety invariants already proven against T-Invest.

## Required sequence

1. Create Rust workspace and crate boundaries.
2. Pin and qualify the Nautilus Rust/v2 dependency strategy.
3. Recreate configuration/readiness/identity/mutation-safety contracts in Rust.
4. Implement shared T-Invest HTTP/WebSocket transport primitives in Rust.
5. Re-run Q1-equivalent exact instrument/futures-economics qualification in Rust.
6. Re-run Q2-equivalent market-data/reconnect qualification in Rust.
7. Re-run Q3/Q4 execution/UNKNOWN/restart semantics through Rust before production execution work is accepted.
8. Remove or archive the transitional Python production package only after Rust parity is demonstrated.
9. Resume #7 from the accepted Rust foundation.

## Proposed workspace

```text
Cargo.toml
crates/
  vox-core/            application composition/runtime
  vox-domain/          broker-neutral domain/application contracts
  vox-tinvest/         T-Invest adapter
  vox-nautilus/        explicit Vox <-> Nautilus mappings
  vox-persistence/     durable state/persistence integration
  vox-api/             HTTP/WebSocket application API
```

Initial migration may start with fewer crates if boundaries stay explicit; do not create empty crate ceremony without a concrete dependency boundary.

## Rust baseline

- stable Rust toolchain, pinned by `rust-toolchain.toml`;
- Cargo workspace with workspace-level dependency/lint/profile policy;
- Tokio for async runtime unless qualification proves another runtime is required;
- `rustls`-based TLS preferred for reproducible deployment;
- `serde` for provider/application serialization boundaries;
- `tracing` for structured logs/correlation;
- exact/fixed-point financial types; no `f32/f64` for broker economics;
- bounded channels only;
- secrets excluded from Debug/Display/logging;
- `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --workspace` are mandatory gates.

## Nautilus dependency gate

The implementation agent must validate the current official Nautilus pure-Rust/v2 surface before committing to crate versions/features. Record:

- exact Nautilus crate/version or git revision;
- required feature flags including precision mode;
- required live/data/execution/model crates;
- capability gaps relative to our already-qualified Python/v1 path;
- upgrade policy and compatibility-test requirement.

Do not silently fall back to a Python runtime inside Vox Core. Any temporary v2 gap requiring Python must be escalated to Head of Development before implementation.

## Migration acceptance

Rust foundation is accepted when:

- Vox Core builds/runs without CPython;
- config defaults to sandbox and live mutation remains fail-closed;
- broker/client/exchange identities remain distinct;
- `UNKNOWN` mutation outcome is preserved;
- T-Invest HTTPS and WebSocket connections work in Rust;
- exact Q1 economics pass without approximation;
- reconnect/subscription behavior passes;
- restart/reconciliation semantics have an executable Rust path or a documented gate before execution issue #10;
- no unbounded queues or idle busy loop are introduced;
- Python remains available only as isolated research/ML worker technology.
