import { describe, expect, it } from "vitest";
import { VoxApiError, type ApiError, type ErrorCategory } from "@vox/api-client";

import {
  capabilityUnavailableInfo,
  dataOf,
  fromApiError,
  isCapabilityUnavailable,
  isFailure,
  isUnknown,
  ready,
  type DataState,
} from "./data-state";

function envelope(
  category: ErrorCategory,
  details?: unknown,
): ApiError {
  const error: ApiError = {
    code: category,
    message: category,
    correlation_id: "corr",
    category,
    retryable: category === "TRANSIENT",
  };
  if (details !== undefined) error.details = details;
  return error;
}

describe("fromApiError", () => {
  it("maps PERMISSION to PERMISSION_DENIED", () => {
    const state = fromApiError(envelope("PERMISSION"));
    expect(state.kind).toBe("PERMISSION_DENIED");
    expect(isFailure(state)).toBe(true);
    expect(isUnknown(state)).toBe(false);
  });

  it("maps AUTHENTICATION with the category still visible", () => {
    const body = envelope("AUTHENTICATION");
    const state = fromApiError(new VoxApiError(401, body));
    expect(state.kind).toBe("PERMISSION_DENIED");
    expect(isFailure(state)).toBe(true);
    if (state.kind === "PERMISSION_DENIED") {
      expect(state.error.category).toBe("AUTHENTICATION");
    }
  });

  it("maps UNRESOLVED_UNKNOWN to UNKNOWN, never ERROR", () => {
    const state = fromApiError(envelope("UNRESOLVED_UNKNOWN"));
    expect(state.kind).toBe("UNKNOWN");
    expect(state.kind).not.toBe("ERROR");
    expect(isUnknown(state)).toBe(true);
    expect(isFailure(state)).toBe(false);
  });

  it("keeps last data on STALE and carries age_ms", () => {
    const previous = ready({ quote: "272.550000000" });
    const state = fromApiError(
      envelope("STALE", { age_ms: 1500 }),
      previous,
    );
    expect(state).toEqual({
      kind: "STALE",
      data: { quote: "272.550000000" },
      age_ms: 1500,
    });
  });

  it("does not pretend CAPABILITY_UNAVAILABLE is READY", () => {
    const state = fromApiError(
      envelope("CAPABILITY_UNAVAILABLE", {
        capability: "RISK_VERDICT",
        owner: "#21",
      }),
    );
    expect(state.kind).not.toBe("READY");
    expect(isCapabilityUnavailable(state)).toBe(true);
    expect(isFailure(state)).toBe(false);
    expect(capabilityUnavailableInfo(state)).toEqual({
      capability: "RISK_VERDICT",
      owner: "#21",
    });
  });

  it("maps TRANSIENT to RECONNECTING when last data exists", () => {
    const previous: DataState<{ n: number }> = ready({ n: 1 });
    const state = fromApiError(envelope("TRANSIENT"), previous);
    expect(state).toEqual({ kind: "RECONNECTING", data: { n: 1 } });
  });

  it("maps TRANSIENT without data to LOADING, not ERROR", () => {
    const state = fromApiError(envelope("TRANSIENT"));
    expect(state.kind).toBe("LOADING");
    expect(isFailure(state)).toBe(false);
  });

  it("maps INTERNAL to ERROR while keeping last data", () => {
    const state = fromApiError(envelope("INTERNAL"), ready({ n: 2 }));
    expect(state.kind).toBe("ERROR");
    expect(dataOf(state)).toEqual({ n: 2 });
    if (state.kind === "ERROR") {
      expect(state.error.category).toBe("INTERNAL");
    }
  });

  it("maps STALE without previous data to STALE, not ERROR", () => {
    const state = fromApiError(envelope("STALE", { age_ms: 40 }));
    expect(state.kind).toBe("STALE");
    expect(isFailure(state)).toBe(false);
    expect(dataOf(state)).toBeUndefined();
    if (state.kind === "STALE") expect(state.age_ms).toBe(40);
  });

  it("maps VALIDATION to ERROR", () => {
    const state = fromApiError(envelope("VALIDATION"));
    expect(state.kind).toBe("ERROR");
    if (state.kind === "ERROR") expect(state.error.category).toBe("VALIDATION");
  });

  it("maps NOT_FOUND to ERROR", () => {
    const state = fromApiError(envelope("NOT_FOUND"));
    expect(state.kind).toBe("ERROR");
    if (state.kind === "ERROR") expect(state.error.category).toBe("NOT_FOUND");
  });

  it("maps CONFLICT to ERROR", () => {
    const state = fromApiError(envelope("CONFLICT"));
    expect(state.kind).toBe("ERROR");
    if (state.kind === "ERROR") expect(state.error.category).toBe("CONFLICT");
  });

  it("maps CAPABILITY_UNAVAILABLE without details to DEGRADED", () => {
    const state = fromApiError(envelope("CAPABILITY_UNAVAILABLE"));
    expect(state.kind).toBe("DEGRADED");
    expect(isCapabilityUnavailable(state)).toBe(true);
    expect(isFailure(state)).toBe(false);
    expect(capabilityUnavailableInfo(state)).toEqual({});
  });
});

describe("UNKNOWN vs failure", () => {
  it("does not treat UNKNOWN as a generic failure", () => {
    const state: DataState<string> = fromApiError(
      envelope("UNRESOLVED_UNKNOWN"),
      ready("last"),
    );
    expect(isUnknown(state)).toBe(true);
    expect(isFailure(state)).toBe(false);
    expect(dataOf(state)).toBe("last");
    expect(state.kind).toBe("UNKNOWN");
  });
});
