import { describe, expect, it } from "vitest";
import type { BrokerConnectionMetadataDto, ConnectionDetailsDto, RuntimeHealthDto } from "@vox/api-client";
import { AccountStore } from "../account";
import { VoxService } from "../vox";
import { establishAndLoadPlatform } from "./load";

const connection: BrokerConnectionMetadataDto = {
  connection_id: "conn-real",
  provider: "T_INVEST",
  environment: "PRODUCTION",
  display_label: "Primary broker",
  enabled: true,
  credential_status: "VALID",
  credential_class: "FULL_ACCESS",
  credential_scope: "ALL_ACCESSIBLE_ACCOUNTS",
  capabilities: ["PORTFOLIO_READ", "PRODUCTION_ORDERS_PROVIDER_ALLOWED"],
  health: { state: "HEALTHY", reason_code: "NONE", retryable: false },
  created_at_unix_ms: 1,
  updated_at_unix_ms: 2,
};

const details: ConnectionDetailsDto = {
  connection,
  accounts: [{
    connection_id: "conn-real",
    provider: "T_INVEST",
    environment: "PRODUCTION",
    provider_account_id: "provider-4417",
    display_name: "Capital account",
    account_type: "BROKER",
    account_status: "OPEN",
    access_level: "FULL_ACCESS",
    accessible: true,
    capabilities: ["PORTFOLIO_READ", "PRODUCTION_ORDERS_PROVIDER_ALLOWED"],
    discovered_at_unix_ms: 3,
  }],
  bindings: [{
    binding_id: "binding-1",
    connection_id: "conn-real",
    provider: "T_INVEST",
    environment: "PRODUCTION",
    provider_account_id: "provider-4417",
    account_id: "account-real",
    enabled: true,
    created_at_unix_ms: 4,
    updated_at_unix_ms: 5,
  }],
  execution_authorizations: [{
    connection_id: "conn-real",
    provider_account_id: "provider-4417",
    mode: "MANUAL_ALLOWED",
    authorization_revision: 7,
    changed_by: "admin",
    changed_at_unix_ms: 6,
  }],
};

const runtime: RuntimeHealthDto = {
  state: "READY",
  reason_code: "RECONCILIATION_COMPLETE",
  reason: "ready",
  provider: "T_INVEST",
  environment: "PRODUCTION",
  account_display: "Capital account",
  runtime_epoch: 9,
  connected: true,
  unresolved_unknown_count: 0,
  open_order_count: 0,
  active_stop_count: 0,
  stream_states: [],
  persistence_healthy: true,
  execution_authorized: true,
  new_exposure_allowed: true,
};

describe("platform loader", () => {
  it("joins real session, connection, binding and authorization through generated client", async () => {
    const paths: string[] = [];
    const fetchImpl: typeof fetch = async (input) => {
      const url = new URL(String(input), "http://localhost");
      paths.push(url.pathname);
      const bodies: Record<string, unknown> = {
        "/api/v1/auth/session": {
          user_id: "operator-1",
          effective_permissions: ["VIEW_CONNECTION_METADATA", "SUBMIT_PRODUCTION_MANUAL_ORDERS"],
          csrf_token: "csrf-real",
          expires_at_unix_ms: 99,
        },
        "/api/v1/broker-connections": [connection],
        "/api/v1/broker-connections/conn-real": details,
        "/api/v1/runtime": runtime,
      };
      return new Response(JSON.stringify(bodies[url.pathname]), { status: 200, headers: { "content-type": "application/json" } });
    };
    const result = await establishAndLoadPlatform(new VoxService(new AccountStore(), { fetch: fetchImpl }), "secret");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.session.userId).toBe("operator-1");
    expect(result.value.session.effectivePermissions.has("SUBMIT_PRODUCTION_MANUAL_ORDERS")).toBe(true);
    expect(result.value.accounts).toHaveLength(1);
    expect(result.value.accounts[0]).toMatchObject({
      accountDisplay: "Capital account",
      providerAccountId: "provider-4417",
      scope: { broker_connection_id: "conn-real", account_id: "account-real", trading_mode: "LIVE" },
      executionAuthorization: { mode: "MANUAL_ALLOWED" },
    });
    expect(paths).toEqual(expect.arrayContaining([
      "/api/v1/auth/session",
      "/api/v1/broker-connections",
      "/api/v1/broker-connections/conn-real",
      "/api/v1/runtime",
    ]));
  });
});
