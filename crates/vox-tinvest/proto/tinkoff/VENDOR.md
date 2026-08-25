# Official T-Invest contracts

Source: `https://opensource.tbank.ru/invest/invest-contracts.git`

Pinned revision: `762e720e27164213f41cac0b226c5698c2ae8199`

Revision date: `2026-07-31T13:03:04Z`

Vendored files:

- `common.proto`
- `instruments.proto`
- `marketdata.proto`
- `operations.proto`
- `orders.proto` (required SandboxService import; execution remains out of scope)
- `sandbox.proto`
- `stoporders.proto` (required SandboxService import; execution remains out of scope)
- `users.proto`
- `google/api/field_behavior.proto`

SHA-256:

- `common.proto`: `15ae831b2aed864abae862140a33738341a132dd3ef3369ec0311cae231f729f`
- `instruments.proto`: `d167f7ae3ab680f589a4f74c40cc97e6b4325bdf8ec42eb10fa5a90eb75e8eec`
- `marketdata.proto`: `60474e266b4e9f7c7a228728dc6da7bfa516a5cac4f85739e624d3ce4bebb263`
- `operations.proto`: `ecef47d34e3ab29f9d6decf7cd2cff1a618199f89a9e2cb5adffecae8abccad1`
- `orders.proto`: `72d994d00ae6573a5b442bcd387e082435e0ec1f41e98a6f4893496427147b22`
- `sandbox.proto`: `4db81cdc7b5d172b2e2a8e8c5cd8d5adbac6ca3732932e8d594e3f6191052153`
- `stoporders.proto`: `5da3cfea3c020feabf13ae9a3e4c42c1482a2e7eef44a21c624236537d0ca8c7`
- `users.proto`: `198d96b1f36238654508d509b81afe0a975c368c07089bd269d9c37dded7ee20`
- `google/api/field_behavior.proto`: `d6e56bfb1cede233ff3a62fed8ed1512af76ce234b3fee7a2ca5fdeab4571f9a`

Rust provider types and all services imported by the pinned contracts are generated during build.
Only capability-inventory-approved adapters may dispatch mutation methods.
Update requires an explicit revision change plus contract inventory tests.
