# Broker connection and credential security

Issue #17 owns provider-neutral connection metadata, encrypted credential storage, account
discovery/binding, RBAC and Vox execution authorization. External HTTP/WebSocket transport remains
#38. Provider execution state machines remain #10. Runtime reconciliation remains #11.

## Canonical ownership

```text
BrokerConnection(provider, environment, CredentialRef)
  -> BrokerAccount[]
       -> BrokerAccountBinding(VoxAccountId)
       -> ExecutionAuthorization(default disabled)

CredentialRef -> SecretStore -> encrypted_credentials(payload ciphertext + wrapped DEK)
                           KEK -> KeyProvider (outside database)
```

`vox-connections` owns generic types and lifecycle. `vox-tinvest::connection_provider` owns
T-Invest endpoint routing, credential validation and provider-account capability mapping.
Read-side resolution (`resolve_read_credential`) returns internal credential material only when
connection, provider environment, broker account and explicit Vox account binding all match.
Execution-side resolution (`resolve_execution_credential`) additionally fails closed unless Vox
authorization permits the requested mutation purpose. A full-access provider token never decrypts
for mutation while `ExecutionAuthorization` is `Disabled`. Transport/runtime client replacement
remains deferred; no cached authorization bridge is exposed.

## Secret envelope

- AEAD: AES-256-GCM through `ring`.
- Random 256-bit DEK per credential encryption. DEK encrypts token with AES-256-GCM.
- External KEK wraps DEK with separate AES-256-GCM operation and random 96-bit nonce. Payload and
  DEK wrapping use distinct nonces and AAD purposes.
- Authenticated associated data binds credential reference, provider, environment, algorithm and
  key version. Cross-environment row substitution fails authentication or context validation.
- SQLite stores payload ciphertext, wrapped DEK, both nonces, algorithm, key version, timestamps
  and short SHA-256 fingerprint only. It never stores KEK, plaintext DEK or plaintext token.
- `KeyProvider` supplies 32-byte KEKs by version. `StaticKeyProvider::from_hex_environment` loads
  `current_version` plus every retained older version from external environment variables. These
  variables belong in process/service secret configuration, never application DB or source.
- `rewrap` decrypts with recorded version and encrypts with current version. Retire old KEK only
  after all rows rewrap and qualification succeeds.
- `SecretBytes`, `KeyMaterial` and T-Invest `SecretToken` zeroize owned memory. `Debug`/`Display`
  redact secret values. Secret types have no serde implementation.
- Missing/wrong KEK, modified ciphertext, modified context and unsupported envelope fail closed.

DB backup without external KEK reveals metadata and ciphertext, not broker credentials.

## Lifecycle and safety

1. Authenticated actor with `MANAGE_CREDENTIALS` submits credential once over Vox TLS transport.
2. Provider adapter routes by typed environment and validates against matching provider endpoint.
3. Adapter discovers every accessible account and provider facts. Only then service writes
   encrypted secret and non-secret metadata.
4. One `CredentialRef` belongs to connection; discovered accounts never duplicate secret.
5. Every account receives explicit `ExecutionAuthorization::Disabled`, including provider
   full-access credentials. `ManualAllowed` and `AutomatedAllowed` require separate Vox RBAC.
6. Credential rotation validates new material, closes connection health to `VALIDATING`, replaces
   envelope, persists broker facts, and returns `reconnect_required=true`. Failure compensates old
   secret/metadata or disables credential. Future transport integration must reconnect and
   reconcile before using changed credential.
7. Disable closes connection use. Delete requires disabled state, writes a `PENDING_DELETE`
   tombstone with audit, then deletes the secret, then removes metadata with audit. If secret
   deletion fails, the tombstone remains and resolution stays closed. Missing secret always
   prevents connection resolution. RBAC, binding and authorization mutations persist state and
   audit in one database transaction.
8. Rediscovery preserves missing accounts as inaccessible and marks health
   `ACCOUNT_ACCESS_CHANGED`. Exact binding resolution then fails closed.

Sandbox credentials construct only sandbox clients; production credentials construct only
production clients. `BrokerEnvironment` contains only `SANDBOX` and `PRODUCTION`; paper trading is
a separate future `TradingMode`, never broker routing.

Credential class is confirmed only from authoritative onboarding context or broker facts.
T-Invest `GetAccounts.access_level` is retained as an account capability fact, not treated as token
class introspection. Production credential class and scope remain `UNKNOWN`/`NOT_CONFIRMED`;
sandbox class is proven by explicit contour onboarding. Transfer access is never inferred.

## RBAC

Permissions stay separate, server-enforced values:

- view connection metadata;
- manage credentials;
- disable/delete connection;
- emergency halt, which can always reduce or disable live execution without the enable privilege;
- discover accounts;
- bind accounts;
- view portfolio;
- submit sandbox orders;
- submit production manual orders;
- enable automated production execution;
- change risk policy;
- emergency halt;
- security administration.

No `admin=true` bypass exists. Disabled/missing users have no effective permissions. Credential,
binding, connection-state and production-authorization changes append audit rows containing actor,
time, correlation ID and safe previous/new state. Audit schema has no secret field.

Strategy/AI identities receive only assigned roles. Without
`ENABLE_AUTOMATED_PRODUCTION_EXECUTION`, they cannot elevate authorization.

## Internal API contract

`ConnectionService` exposes transport-neutral operations for create/list, validate/rediscover,
rotate, disable/delete, bind/unbind, authorization and exact bound credential resolution. Read models
serialize credential references and fingerprints, never credential material.

Typed failures distinguish permission denial, invalid/inactive credential, insufficient provider
permission, wrong environment, provider outage, account-access change, missing binding and disabled
authorization. #38 maps these domain failures to external API errors without provider wire DTOs.

`All accounts` aggregation remains read-only. No method accepts an aggregate target for execution.

## Official contract verification

Pinned contract source: `crates/vox-tinvest/proto/tinkoff/VENDOR.md`, upstream
`invest-contracts`, revision `762e720e27164213f41cac0b226c5698c2ae8199` dated
2026-07-31.

- [Token types and authorization](https://developer.tbank.ru/invest/intro/intro/token): token is
  sent as authorization metadata; read-only, full-access, transfer-access, single-account and
  sandbox token classes exist. API does not provide a reliable token-class introspection RPC.
  Vox stores `UNKNOWN`/`NOT_CONFIRMED` unless onboarding or returned account access facts prove a
  narrower statement. Broker permission never enables Vox execution authorization.
- [Protocols and endpoints](https://developer.tbank.ru/invest/intro/developer/protocols/): production
  endpoint is `invest-public-api.tbank.ru:443`; sandbox endpoint is
  `sandbox-invest-public-api.tbank.ru:443`. Vox selects exactly one endpoint from
  `BrokerEnvironment`; no fallback or contour probing.
- [Sandbox](https://developer.tbank.ru/invest/intro/developer/sandbox/) and
  [sandbox URL differences](https://developer.tbank.ru/invest/intro/developer/sandbox/url_difference):
  sandbox uses separate endpoint and token. Pinned `sandbox.proto` exposes
  `SandboxService.GetSandboxAccounts(GetAccountsRequest) -> GetAccountsResponse`; adapter calls
  existing `TInvestGrpcClient::sandbox` and `AccountReadClient::sandbox_accounts`.
- [UsersService/GetAccounts](https://developer.tbank.ru/invest/api/users-service-get-accounts),
  [accounts overview](https://developer.tbank.ru/invest/services/accounts/head-account), and
  [users methods](https://developer.tbank.ru/invest/services/accounts/users): pinned `users.proto`
  exposes `UsersService.GetAccounts(GetAccountsRequest) -> GetAccountsResponse`. Response account
  fields used: `id`, `type`, `name`, `status`, `opened_date`, `closed_date`, `access_level`.
  Returned list is broker-authoritative accessible-account discovery; Vox never assumes one token
  equals one account and never auto-binds accounts.
- [Deadlines](https://developer.tbank.ru/invest/intro/developer/deadlines): `GetAccounts` and
  `GetSandboxAccounts` recommended minimum deadline is 300 ms. Existing shared gRPC client owns
  unary deadlines; this adapter creates no second client or retry policy.

Known doc/proto difference: public docs describe token categories but pinned protobuf exposes no
token-category or single-account-restriction introspection field. Vox decision: preserve class or
scope as unknown/not confirmed; never infer restriction from returned account count. Wrong-contour
token rejection is not distinguishable from invalid credential through `GetAccounts`; Vox keeps
typed requested environment, reports credential rejection, and never tries another endpoint.

## Phase boundary

Phase A contains Rust domain/application ports, secure persistence, T-Invest validation adapter,
bound credential resolution and tests. PR #45 owns REST/WebSocket/OpenAPI/public DTO/generated
TypeScript/frontend integration. No Phase B transport wiring exists here.

## Qualification

Committed cases live in `qualification/broker_connection_security_contracts.json`. Deterministic
tests cover plaintext absence, DB-only compromise, wrong/corrupt KEK/ciphertext, key rewrap,
redaction, multi-account selective binding, multiple same-provider connections, default-off
production authorization, RBAC denial, access revocation and frozen exact target. Ignored live test
`connection_provider_live` performs read-only T-Invest sandbox validation and account discovery.
