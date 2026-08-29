import { describe, expect, it } from "vitest";
import type { ExecutionScope, InstrumentIdentityDto } from "@vox/api-client";
import { AccountStore } from "../account/store";
import { createWidget } from "../ui/widget";
import { InstrumentContextStore } from "./store";
import {
  instrumentRefFromIdentity,
  sameInstrumentIdentity,
  type InstrumentRef,
} from "./types";

function sber(overrides: Partial<InstrumentIdentityDto> = {}): InstrumentIdentityDto {
  return {
    provider: "T_INVEST",
    uid: "BBG004730N88",
    ticker: "SBER",
    class_code: "TQBR",
    figi: "BBG004730N88",
    ...overrides,
  };
}

function si(overrides: Partial<InstrumentIdentityDto> = {}): InstrumentIdentityDto {
  return {
    provider: "T_INVEST",
    uid: "FUTSI0326000",
    ticker: "Si",
    class_code: "SPBFUT",
    figi: "FUTSI0326000",
    ...overrides,
  };
}

describe("InstrumentContextStore", () => {
  it("defaults widgets to LINKED and resolves the global instrument", () => {
    const store = new InstrumentContextStore();
    const sberRef = instrumentRefFromIdentity(sber());

    expect(store.global()).toBeNull();
    expect(store.widgetMode("chart-a")).toBe("LINKED");
    expect(store.resolve("chart-a")).toBeNull();

    store.setGlobal(sberRef);
    expect(store.widgetMode("chart-a")).toBe("LINKED");
    expect(store.resolve("chart-a")).toEqual(sberRef);
    expect(store.global()).toEqual(sberRef);
  });

  it("linked widgets follow global instrument changes", () => {
    const store = new InstrumentContextStore();
    const sberRef = instrumentRefFromIdentity(sber());
    const siRef = instrumentRefFromIdentity(si());

    store.setGlobal(sberRef);
    expect(store.resolve("chart-a")).toEqual(sberRef);
    expect(store.resolve("tape")).toEqual(sberRef);

    store.setGlobal(siRef);
    expect(store.widgetMode("chart-a")).toBe("LINKED");
    expect(store.resolve("chart-a")).toEqual(siRef);
    expect(store.resolve("tape")).toEqual(siRef);
    expect(store.global()).toEqual(siRef);
  });

  it("pinned widgets stay on the pinned instrument when global changes", () => {
    const store = new InstrumentContextStore();
    const sberRef = instrumentRefFromIdentity(sber());
    const siRef = instrumentRefFromIdentity(si());

    store.setGlobal(sberRef);
    store.pin("chart-b", siRef);

    expect(store.widgetMode("chart-b")).toBe("PINNED");
    expect(store.resolve("chart-b")).toEqual(siRef);
    expect(store.widgetMode("chart-a")).toBe("LINKED");
    expect(store.resolve("chart-a")).toEqual(sberRef);

    store.setGlobal(instrumentRefFromIdentity(sber({ ticker: "SBERP" })));

    expect(store.widgetMode("chart-b")).toBe("PINNED");
    expect(store.resolve("chart-b")).toEqual(siRef);
    expect(store.resolve("chart-b")?.ticker).toBe("Si");
    expect(store.widgetMode("chart-a")).toBe("LINKED");
    expect(store.resolve("chart-a")?.ticker).toBe("SBERP");
    expect(
      sameInstrumentIdentity(store.resolve("chart-a") as InstrumentRef, sberRef),
    ).toBe(true);
  });

  it("unlink returns a widget to LINKED so it follows global again", () => {
    const store = new InstrumentContextStore();
    const sberRef = instrumentRefFromIdentity(sber());
    const siRef = instrumentRefFromIdentity(si());

    store.setGlobal(sberRef);
    store.pin("chart-b", siRef);
    store.unlink("chart-b");

    expect(store.widgetMode("chart-b")).toBe("LINKED");
    expect(store.resolve("chart-b")).toEqual(sberRef);

    store.setGlobal(siRef);
    expect(store.resolve("chart-b")).toEqual(siRef);
  });

  it("treats ticker and class_code as display aliases, never identity", () => {
    const renamed = instrumentRefFromIdentity(
      sber({ ticker: "SBERP", class_code: "TQBR" }),
    );
    const original = instrumentRefFromIdentity(sber());

    expect(sameInstrumentIdentity(original, renamed)).toBe(true);
    expect(original.ticker).not.toBe(renamed.ticker);
    expect(original.uid).toBe(renamed.uid);
    expect(original.provider).toBe(renamed.provider);
  });

  it("freezes pinned refs so later mutation of the input is ignored", () => {
    const store = new InstrumentContextStore();
    const mutable: InstrumentRef = {
      provider: "T_INVEST",
      uid: "FUTSI0326000",
      ticker: "Si",
      class_code: "SPBFUT",
    };

    store.pin("chart-b", mutable);
    (mutable as { ticker: string }).ticker = "RI";

    expect(store.resolve("chart-b")?.ticker).toBe("Si");
  });

  it("notifies subscribers on global, pin, and unlink", () => {
    const store = new InstrumentContextStore();
    let calls = 0;
    const unsubscribe = store.subscribe(() => {
      calls += 1;
    });

    store.setGlobal(instrumentRefFromIdentity(sber()));
    store.pin("chart-b", instrumentRefFromIdentity(si()));
    store.unlink("chart-b");
    unsubscribe();
    store.setGlobal(null);

    expect(calls).toBe(3);
  });

  it("LINKED widget header follows setGlobal; PINNED does not; account switch is independent", () => {
    const store = new InstrumentContextStore();
    const accounts = new AccountStore();
    const sberRef = instrumentRefFromIdentity(sber());
    const siRef = instrumentRefFromIdentity(si());
    store.setGlobal(sberRef);
    store.pin("pinned", siRef);

    const paint = (id: string) =>
      createWidget({
        title: id,
        instrument: store.resolve(id)!,
        mode: store.widgetMode(id),
      });

    const linkedBefore = paint("linked");
    const pinnedBefore = paint("pinned");
    expect(linkedBefore.textContent).toContain("связан");
    expect(linkedBefore.textContent).toContain("SBER");
    expect(pinnedBefore.textContent).toContain("закреплён");
    expect(pinnedBefore.textContent).toContain("Si");
    expect(linkedBefore.classList.contains("is-pinned")).toBe(false);
    expect(pinnedBefore.getAttribute("data-binding")).toBe("PINNED");

    store.setGlobal(
      instrumentRefFromIdentity({
        provider: "T_INVEST",
        uid: "BBG004730RP0",
        ticker: "GAZP",
        class_code: "TQBR",
      }),
    );
    const linkedAfter = paint("linked");
    const pinnedAfter = paint("pinned");
    expect(linkedAfter.textContent).toContain("GAZP");
    expect(pinnedAfter.textContent).toContain("Si");
    expect(pinnedAfter.textContent).not.toContain("GAZP");
    expect(store.resolve("pinned")).toEqual(siRef);

    const scope: ExecutionScope = {
      provider: "T_INVEST",
      environment: "SANDBOX",
      broker_connection_id: "connection:primary",
      account_id: "account:b",
      trading_mode: "LIVE",
    };
    accounts.switchTo(scope);
    expect(store.resolve("linked")?.ticker).toBe("GAZP");
    expect(store.resolve("pinned")).toEqual(siRef);
    expect(store.widgetMode("pinned")).toBe("PINNED");
  });
});
