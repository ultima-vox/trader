import type { Decimal } from "@vox/api-client";

export const NANO_SCALE = 1_000_000_000n;
export const I64_MAX = 9223372036854775807n;
export const I64_MIN = -9223372036854775808n;

const CANONICAL = /^-?(0|[1-9]\d*)\.\d{9}$/;
const TOO_PRECISE = /^-?(0|[1-9]\d*)\.\d{10,}$/;

export type ExactDecimal = {
  readonly nanos: bigint;
};

export class DecimalParseError extends Error {
  override readonly name = "DecimalParseError";
}

export function fromNanos(nanos: bigint): ExactDecimal {
  assertI64Units(nanos);
  return Object.freeze({ nanos });
}

export function parseDecimal(value: Decimal): ExactDecimal {
  if (typeof value !== "string") {
    throw new DecimalParseError(
      "decimal must be a canonical fixed-point string, not exponent, NaN, Infinity, or free-form text",
    );
  }
  if (value.length === 0 || /\s/.test(value)) {
    throw new DecimalParseError("decimal cannot be empty or whitespace");
  }

  const lower = value.toLowerCase();
  if (
    lower.includes("e") ||
    lower.includes("nan") ||
    lower.includes("inf") ||
    value.includes("+")
  ) {
    throw new DecimalParseError(
      "decimal must be a canonical fixed-point string, not exponent, NaN, Infinity, or free-form text",
    );
  }

  if (TOO_PRECISE.test(value)) {
    throw new DecimalParseError(
      "decimal has more precision than the canonical nano scale of nine fraction digits",
    );
  }

  if (!CANONICAL.test(value)) {
    throw new DecimalParseError(
      "decimal must be a canonical fixed-point string, not exponent, NaN, Infinity, or free-form text",
    );
  }

  const negative = value.startsWith("-");
  const unsigned = negative ? value.slice(1) : value;
  const dot = unsigned.indexOf(".");
  const whole = unsigned.slice(0, dot);
  const fraction = unsigned.slice(dot + 1);
  const magnitude = BigInt(whole) * NANO_SCALE + BigInt(fraction);
  return fromNanos(negative ? -magnitude : magnitude);
}

export function formatCanonical(nanos: bigint): string {
  const negative = nanos < 0n;
  const magnitude = negative ? -nanos : nanos;
  const units = magnitude / NANO_SCALE;
  const fraction = magnitude % NANO_SCALE;
  const sign = negative ? "-" : "";
  return `${sign}${units.toString()}.${fraction.toString().padStart(9, "0")}`;
}

export function toCanonical(value: ExactDecimal): Decimal {
  return formatCanonical(value.nanos);
}

export function add(left: ExactDecimal, right: ExactDecimal): ExactDecimal {
  return fromNanos(left.nanos + right.nanos);
}

export function sub(left: ExactDecimal, right: ExactDecimal): ExactDecimal {
  return fromNanos(left.nanos - right.nanos);
}

export function neg(value: ExactDecimal): ExactDecimal {
  return fromNanos(-value.nanos);
}

export function compare(left: ExactDecimal, right: ExactDecimal): -1 | 0 | 1 {
  if (left.nanos < right.nanos) return -1;
  if (left.nanos > right.nanos) return 1;
  return 0;
}

export function formatDisplay(value: ExactDecimal | Decimal): string {
  const canonical = typeof value === "string" ? toCanonical(parseDecimal(value)) : toCanonical(value);
  const negative = canonical.startsWith("-");
  const unsigned = negative ? canonical.slice(1) : canonical;
  const dot = unsigned.indexOf(".");
  const whole = unsigned.slice(0, dot);
  const fraction = unsigned.slice(dot + 1);
  const grouped = groupThousands(whole);
  return `${negative ? "-" : ""}${grouped},${fraction}`;
}

function assertI64Units(nanos: bigint): void {
  const units = nanos / NANO_SCALE;
  const unsigned = units < 0n ? -units : units;
  if (unsigned > I64_MAX) {
    throw new DecimalParseError("decimal units exceed i64");
  }
}

function groupThousands(digits: string): string {
  const characters = digits.split("");
  const parts: string[] = [];
  for (let index = characters.length; index > 0; index -= 3) {
    const start = index - 3 > 0 ? index - 3 : 0;
    parts.push(characters.slice(start, index).join(""));
  }
  return parts.reverse().join(" ");
}
