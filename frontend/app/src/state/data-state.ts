import { VoxApiError, type ApiError, type Capability } from "@vox/api-client";

export type DataStateKind =
  | "LOADING"
  | "EMPTY"
  | "READY"
  | "STALE"
  | "RECONNECTING"
  | "DEGRADED"
  | "PERMISSION_DENIED"
  | "ERROR"
  | "UNKNOWN";

export type DataState<T> =
  | { readonly kind: "LOADING"; readonly data?: T }
  | { readonly kind: "EMPTY" }
  | { readonly kind: "READY"; readonly data: T }
  | { readonly kind: "STALE"; readonly data?: T; readonly age_ms: number }
  | { readonly kind: "RECONNECTING"; readonly data?: T }
  | {
      readonly kind: "DEGRADED";
      readonly data?: T;
      readonly error?: ApiError;
      readonly capability?: Capability;
      readonly owner?: string;
    }
  | {
      readonly kind: "PERMISSION_DENIED";
      readonly error: ApiError;
      readonly data?: T;
    }
  | {
      readonly kind: "ERROR";
      readonly error: ApiError;
      readonly data?: T;
    }
  | {
      readonly kind: "UNKNOWN";
      readonly error?: ApiError;
      readonly data?: T;
    };

const CAPABILITIES: Record<Capability, true> = {
  RUNTIME_HEALTH: true,
  ACCOUNT_READ_SIDE: true,
  ORDER_EXECUTION: true,
  PROTECTION_EXECUTION: true,
  PROTECTION_DEFAULTS: true,
  BULK_PROTECTION_MIGRATION: true,
  CONNECTION_MANAGEMENT: true,
  RBAC: true,
  RISK_VERDICT: true,
  PORTFOLIO_VALUATION: true,
  MARKET_DATA: true,
  STRATEGY: true,
  DECISION: true,
  MACHINE_LEARNING: true,
  RESEARCH: true,
  AGGREGATE_ACCOUNTS: true,
  MULTI_PROVIDER: true,
  NON_LIVE_TRADING_MODE: true,
};

export function loading<T>(data?: T): DataState<T> {
  return data === undefined ? { kind: "LOADING" } : { kind: "LOADING", data };
}

export function empty<T = never>(): DataState<T> {
  return { kind: "EMPTY" };
}

export function ready<T>(data: T): DataState<T> {
  return { kind: "READY", data };
}

export function stale<T>(data: T | undefined, age_ms: number): DataState<T> {
  return data === undefined ? { kind: "STALE", age_ms } : { kind: "STALE", data, age_ms };
}

export function reconnecting<T>(data?: T): DataState<T> {
  return data === undefined
    ? { kind: "RECONNECTING" }
    : { kind: "RECONNECTING", data };
}

export function degraded<T>(data?: T): DataState<T> {
  return data === undefined ? { kind: "DEGRADED" } : { kind: "DEGRADED", data };
}

export function permissionDenied<T>(error: ApiError, data?: T): DataState<T> {
  return data === undefined
    ? { kind: "PERMISSION_DENIED", error }
    : { kind: "PERMISSION_DENIED", error, data };
}

export function error<T>(apiError: ApiError, data?: T): DataState<T> {
  return data === undefined
    ? { kind: "ERROR", error: apiError }
    : { kind: "ERROR", error: apiError, data };
}

export function unknown<T>(apiError?: ApiError, data?: T): DataState<T> {
  const state: {
    kind: "UNKNOWN";
    error?: ApiError;
    data?: T;
  } = { kind: "UNKNOWN" };
  if (apiError !== undefined) state.error = apiError;
  if (data !== undefined) state.data = data;
  return state;
}

export function dataOf<T>(state: DataState<T>): T | undefined {
  if (state.kind === "EMPTY") return undefined;
  if ("data" in state) return state.data;
  return undefined;
}

export function isUnknown(state: DataState<unknown>): boolean {
  return state.kind === "UNKNOWN";
}

export function isFailure(state: DataState<unknown>): boolean {
  return state.kind === "ERROR" || state.kind === "PERMISSION_DENIED";
}

export function isCapabilityUnavailable(state: DataState<unknown>): boolean {
  return (
    state.kind === "DEGRADED" &&
    state.error?.category === "CAPABILITY_UNAVAILABLE"
  );
}

export function capabilityUnavailableInfo(
  state: DataState<unknown>,
): { capability?: Capability; owner?: string } | undefined {
  if (!isCapabilityUnavailable(state) || state.kind !== "DEGRADED") {
    return undefined;
  }
  const info: { capability?: Capability; owner?: string } = {};
  if (state.capability !== undefined) info.capability = state.capability;
  if (state.owner !== undefined) info.owner = state.owner;
  return info;
}

export function fromApiError<T>(
  failure: VoxApiError | ApiError,
  previous?: DataState<T>,
): DataState<T> {
  const apiError = asApiError(failure);
  const previousData = previous === undefined ? undefined : dataOf(previous);

  switch (apiError.category) {
    case "PERMISSION":
    case "AUTHENTICATION":
      return permissionDenied(apiError, previousData);
    case "UNRESOLVED_UNKNOWN":
      return unknown(apiError, previousData);
    case "STALE":
      return stale(previousData, ageMs(apiError.details));
    case "CAPABILITY_UNAVAILABLE":
      return capabilityUnavailableState(apiError, previousData);
    case "TRANSIENT":
      if (previousData !== undefined) {
        return reconnecting(previousData);
      }
      return loading();
    case "VALIDATION":
    case "NOT_FOUND":
    case "CONFLICT":
    case "INTERNAL":
      return error(apiError, previousData);
    default: {
      const _exhaustive: never = apiError.category;
      void _exhaustive;
      return error(apiError, previousData);
    }
  }
}

function asApiError(failure: VoxApiError | ApiError): ApiError {
  return failure instanceof VoxApiError ? failure.body : failure;
}

function capabilityUnavailableState<T>(
  apiError: ApiError,
  previousData: T | undefined,
): DataState<T> {
  const details = readCapabilityDetails(apiError.details);
  const state: {
    kind: "DEGRADED";
    data?: T;
    error?: ApiError;
    capability?: Capability;
    owner?: string;
  } = { kind: "DEGRADED", error: apiError };
  if (previousData !== undefined) state.data = previousData;
  if (details.capability !== undefined) state.capability = details.capability;
  if (details.owner !== undefined) state.owner = details.owner;
  return state;
}

function readCapabilityDetails(details: unknown): {
  capability?: Capability;
  owner?: string;
} {
  if (details === null || typeof details !== "object") {
    return {};
  }
  const record = details as { capability?: unknown; owner?: unknown };
  const result: { capability?: Capability; owner?: string } = {};
  const capability = asCapability(record.capability);
  if (capability !== undefined) result.capability = capability;
  if (typeof record.owner === "string") result.owner = record.owner;
  return result;
}

function asCapability(value: unknown): Capability | undefined {
  if (typeof value !== "string") return undefined;
  if (value in CAPABILITIES) return value as Capability;
  return undefined;
}

function ageMs(details: unknown): number {
  if (details === null || typeof details !== "object") return 0;
  if (!("age_ms" in details)) return 0;
  const value = (details as { age_ms: unknown }).age_ms;
  if (typeof value !== "number" || value !== value) return 0;
  if (value === Infinity || value === -Infinity) return 0;
  return value;
}
