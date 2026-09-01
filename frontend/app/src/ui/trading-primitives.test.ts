import { describe, expect, it, vi } from "vitest";
import type { CapabilitySet, RuntimeHealthDto } from "@vox/api-client";
import { CommandHandle } from "../command";
import type { BrowserSession, PlatformAccount } from "../platform";
import { createCapitalConfirmation, createCommandLifecycle, createOrderTicket, executionGate } from "./trading-primitives";

const account: PlatformAccount = {
  scope: { provider: "T_INVEST", environment: "PRODUCTION", broker_connection_id: "conn-1", account_id: "account-1", trading_mode: "LIVE" },
  connectionLabel: "Primary",
  providerAccountId: "provider-4417",
  accountDisplay: "Capital",
  accessible: true,
  connectionEnabled: true,
  connectionHealth: { state: "HEALTHY", reason_code: "NONE", retryable: false },
  connectionCapabilities: ["PRODUCTION_ORDERS_PROVIDER_ALLOWED"],
  binding: { binding_id: "binding-1", connection_id: "conn-1", provider: "T_INVEST", environment: "PRODUCTION", provider_account_id: "provider-4417", account_id: "account-1", enabled: true, created_at_unix_ms: 1, updated_at_unix_ms: 1 },
  executionAuthorization: { connection_id: "conn-1", provider_account_id: "provider-4417", mode: "MANUAL_ALLOWED", authorization_revision: 1, changed_by: "admin", changed_at_unix_ms: 1 },
};
const session: BrowserSession = { userId: "operator", effectivePermissions: new Set(["SUBMIT_PRODUCTION_MANUAL_ORDERS"]), expiresAtUnixMs: 9, csrfReady: true };
const capabilities: CapabilitySet = { provider: "T_INVEST", environment: "PRODUCTION", account_id: "account-1", supported: ["ORDER_EXECUTION"], unavailable: [{ capability: "PROTECTION_EXECUTION", reason: "not projected", owner: "#23" }, { capability: "RISK_VERDICT", reason: "not projected", owner: "#24" }] };
const runtime: RuntimeHealthDto = { state: "READY", reason_code: "RECONCILIATION_COMPLETE", reason: "ready", provider: "T_INVEST", environment: "PRODUCTION", account_display: "Capital", runtime_epoch: 1, connected: true, unresolved_unknown_count: 0, open_order_count: 0, active_stop_count: 0, stream_states: [], persistence_healthy: true, execution_authorized: true, new_exposure_allowed: true };

describe("trading primitives", () => {
  it("uses backend capability, RBAC, authorization and runtime facts", () => {
    expect(executionGate(account, session, capabilities, runtime)).toEqual({ allowed: true });
    expect(executionGate(account, { ...session, effectivePermissions: new Set() }, capabilities, runtime)).toMatchObject({ allowed: false, reason: "missing permission SUBMIT_PRODUCTION_MANUAL_ORDERS" });
    expect(executionGate(account, session, capabilities, { ...runtime, new_exposure_allowed: false })).toMatchObject({ allowed: false });
  });

  it("keeps Buy and Sell visible with identical disabled reason and frozen target", () => {
    const ticket = createOrderTicket({ account, session, capabilities, runtime, command: new CommandHandle(account.scope, "req-1") });
    const buttons = Array.from(ticket.querySelectorAll<HTMLButtonElement>(".vox-ticket__action"));
    expect(buttons).toHaveLength(2);
    expect(buttons.every((button) => button.disabled)).toBe(true);
    expect(buttons.map((button) => button.querySelector(".vox-ticket__action-note")?.textContent)).toEqual(["instrument required", "instrument required"]);
    expect(ticket.querySelector(".vox-ticket__target-lock")?.getAttribute("title")).toContain("req-1");
    expect(ticket.textContent).toContain("#23");
    expect(ticket.textContent).toContain("#24");
  });

  it("requires exact typed phrase for capital confirmation", () => {
    const onConfirm = vi.fn();
    const confirmation = createCapitalConfirmation({ title: "Delete", consequence: "Permanent", phrase: "DELETE account-1", onConfirm });
    const input = confirmation.querySelector("input")!;
    const button = confirmation.querySelector("button")!;
    input.value = "DELETE account";
    input.dispatchEvent(new Event("input"));
    expect(button.disabled).toBe(true);
    input.value = "DELETE account-1";
    input.dispatchEvent(new Event("input"));
    button.click();
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("keeps UNKNOWN state and frozen target visible after active account changes", () => {
    const receipt = {
      logical_request_id: "req-unknown",
      scope: account.scope,
      kind: "POST_ORDER" as const,
      state: "UNKNOWN_AFTER_DISPATCH" as const,
      decision: "RECONCILE" as const,
      correlation_id: "corr-1",
      runtime_epoch: 2,
      created_at_unix_ms: 1,
      updated_at_unix_ms: 2,
    };
    const command = new CommandHandle(account.scope, "req-unknown", receipt, {
      providerAccountId: "provider-4417",
      accountDisplay: "Capital",
      connectionLabel: "Primary",
    });
    const other = { ...account, scope: { ...account.scope, account_id: "account-2" }, accountDisplay: "Other" };
    const status = createCommandLifecycle(other, command);
    expect(status.classList.contains("vox-recon")).toBe(true);
    expect(status.textContent).toContain("UNKNOWN_AFTER_DISPATCH");
    expect(status.textContent).toContain("Capital");
    expect(status.textContent).not.toContain("Other");
    expect(status.querySelector(".vox-ticket__target")?.classList.contains("is-mismatch")).toBe(true);
  });
});
