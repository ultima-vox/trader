import type { CapabilitySet, RuntimeHealthDto } from "@vox/api-client";
import { describe, expect, it } from "vitest";
import { createCapabilityGate, findUnavailable } from "./capability-gate";

const blocked: CapabilitySet = {
  provider: "T_INVEST",
  environment: "SANDBOX",
  supported: ["RUNTIME_HEALTH"],
  unavailable: [
    {
      capability: "ORDER_EXECUTION",
      reason: "нет порта исполнения",
      owner: "#10",
    },
  ],
};

const supported: CapabilitySet = {
  provider: "T_INVEST",
  environment: "SANDBOX",
  supported: ["RUNTIME_HEALTH", "ORDER_EXECUTION"],
  unavailable: [],
};

function health(overrides: Partial<RuntimeHealthDto> = {}): RuntimeHealthDto {
  return {
    state: "READY",
    reason_code: "RECONCILIATION_COMPLETE",
    reason: "ready",
    provider: "T_INVEST",
    environment: "SANDBOX",
    account_display: "Основной",
    runtime_epoch: 1,
    connected: true,
    unresolved_unknown_count: 0,
    open_order_count: 0,
    active_stop_count: 0,
    stream_states: [],
    persistence_healthy: true,
    execution_authorized: true,
    new_exposure_allowed: true,
    ...overrides,
  };
}

describe("CapabilityGate", () => {
  it("keeps unavailable capability disabled and does not fire child clicks", () => {
    let fired = 0;
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Купить";
    button.addEventListener("click", () => {
      fired += 1;
    });

    const gate = createCapabilityGate({
      capabilities: blocked,
      capability: "ORDER_EXECUTION",
      children: [button],
    });
    document.body.append(gate);

    expect(gate.classList.contains("vox-deferred")).toBe(true);
    expect(gate.textContent).toContain("#10");
    expect(gate.textContent).toContain("нет порта исполнения");
    expect(gate.textContent).toContain("Исполнение заявок");
    expect(gate.textContent).toContain("ORDER_EXECUTION");
    expect(button.disabled).toBe(true);
    expect(button.getAttribute("aria-disabled")).toBe("true");
    expect(button.classList.contains("is-disabled")).toBe(true);

    button.click();
    gate.click();
    expect(fired).toBe(0);
    gate.remove();
  });

  it("passes through when capability is supported and runtime allows exposure", () => {
    let fired = 0;
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Купить";
    button.addEventListener("click", () => {
      fired += 1;
    });
    const gate = createCapabilityGate({
      capabilities: supported,
      capability: "ORDER_EXECUTION",
      runtime: health(),
      children: [button],
    });
    document.body.append(gate);
    expect(gate.classList.contains("vox-deferred")).toBe(false);
    expect(button.disabled).toBe(false);
    button.click();
    expect(fired).toBe(1);
    gate.remove();
  });

  it("defers RISK_VERDICT missing from both arrays without minting owner or backend reason", () => {
    const set: CapabilitySet = {
      provider: "T_INVEST",
      environment: "SANDBOX",
      supported: ["RUNTIME_HEALTH"],
      unavailable: [],
    };
    expect(findUnavailable(set, "RISK_VERDICT")).toBeNull();
    const gate = createCapabilityGate({
      capabilities: set,
      capability: "RISK_VERDICT",
    });
    expect(gate.classList.contains("vox-deferred")).toBe(true);
    expect(gate.classList.contains("is-error")).toBe(false);
    expect(gate.textContent).not.toContain("#");
    expect(gate.textContent).not.toContain("возможность не входит в supported");
    expect(gate.textContent).toContain("Вердикт риска");
    expect(gate.textContent).toContain("RISK_VERDICT");
  });

  it("refuses ORDER_EXECUTION when execution_authorized is false", () => {
    const button = document.createElement("button");
    button.type = "button";
    const gate = createCapabilityGate({
      capabilities: supported,
      capability: "ORDER_EXECUTION",
      runtime: health({ execution_authorized: false }),
      children: [button],
    });
    expect(gate.classList.contains("vox-deferred")).toBe(true);
    expect(gate.textContent).toContain("исполнение Vox выключено");
    expect(button.disabled).toBe(true);
  });

  it("fails closed when runtime is omitted for ORDER_EXECUTION", () => {
    const button = document.createElement("button");
    button.type = "button";
    const gate = createCapabilityGate({
      capabilities: supported,
      capability: "ORDER_EXECUTION",
      children: [button],
    });
    expect(gate.classList.contains("vox-deferred")).toBe(true);
    expect(button.disabled).toBe(true);
  });

  it("refuses ORDER_EXECUTION when new_exposure_allowed is false", () => {
    const button = document.createElement("button");
    button.type = "button";
    const gate = createCapabilityGate({
      capabilities: supported,
      capability: "ORDER_EXECUTION",
      runtime: health({ execution_authorized: true, new_exposure_allowed: false }),
      children: [button],
    });
    expect(gate.classList.contains("vox-deferred")).toBe(true);
    expect(gate.textContent).toContain("новая экспозиция запрещена");
    expect(button.disabled).toBe(true);
  });

  it("disables nested textarea", () => {
    const wrap = document.createElement("div");
    const area = document.createElement("textarea");
    wrap.append(area);
    const gate = createCapabilityGate({
      capabilities: blocked,
      capability: "ORDER_EXECUTION",
      children: [wrap],
    });
    expect(area.disabled).toBe(true);
    expect(area.getAttribute("aria-disabled")).toBe("true");
    gate.remove();
  });
});
