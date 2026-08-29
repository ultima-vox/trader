import { describe, expect, it } from "vitest";

import * as decimal from "./exact";
import {
  DecimalParseError,
  I64_MAX,
  I64_MIN,
  add,
  compare,
  formatCanonical,
  formatDisplay,
  fromNanos,
  neg,
  parseDecimal,
  sub,
  toCanonical,
} from "./exact";

describe("parseDecimal", () => {
  it("round-trips a canonical wire string", () => {
    const parsed = parseDecimal("272.550000000");
    expect(parsed.nanos).toBe(272550000000n);
    expect(toCanonical(parsed)).toBe("272.550000000");
  });

  it("accepts a signed canonical value", () => {
    const parsed = parseDecimal("-3140.700000000");
    expect(parsed.nanos).toBe(-3140700000000n);
    expect(toCanonical(parsed)).toBe("-3140.700000000");
  });

  it("accepts canonical zero", () => {
    expect(parseDecimal("0.000000000").nanos).toBe(0n);
    expect(toCanonical(parseDecimal("-0.000000000"))).toBe("0.000000000");
  });

  it("rejects exponent, NaN, Infinity, whitespace, leading plus, leading zeros, extra precision, empty", () => {
    const invalid = [
      "",
      " ",
      " 1.000000000",
      "1.000000000\n",
      "abc",
      "1e9",
      "1E-3",
      "NaN",
      "Infinity",
      "-Infinity",
      "+1.000000000",
      "01.000000000",
      "1.0",
      "1.",
      ".500000000",
      "1.0000000000",
      "1.1234567891",
    ];
    for (const value of invalid) {
      expect(() => parseDecimal(value)).toThrow(DecimalParseError);
    }
    expect(() => parseDecimal(1 as unknown as string)).toThrow(DecimalParseError);
  });

  it("accepts signed i64 unit bounds and rejects one step outside", () => {
    const max = parseDecimal("9223372036854775807.000000000");
    const min = parseDecimal("-9223372036854775808.000000000");
    expect(toCanonical(max)).toBe("9223372036854775807.000000000");
    expect(toCanonical(min)).toBe("-9223372036854775808.000000000");
    expect(fromNanos(I64_MAX * 1_000_000_000n).nanos).toBe(I64_MAX * 1_000_000_000n);
    expect(fromNanos(I64_MIN * 1_000_000_000n).nanos).toBe(I64_MIN * 1_000_000_000n);
    expect(() => parseDecimal("9223372036854775808.000000000")).toThrow(DecimalParseError);
    expect(() => parseDecimal("-9223372036854775809.000000000")).toThrow(DecimalParseError);
    expect(() => fromNanos((I64_MAX + 1n) * 1_000_000_000n)).toThrow(DecimalParseError);
    expect(() => fromNanos((I64_MIN - 1n) * 1_000_000_000n)).toThrow(DecimalParseError);
    expect(() => add(max, parseDecimal("1.000000000"))).toThrow(DecimalParseError);
    expect(() => neg(min)).toThrow(DecimalParseError);
    expect(() => neg(max)).not.toThrow();
  });
});

describe("formatCanonical", () => {
  it("formats a total-nanos equivalent with nine fraction digits", () => {
    expect(formatCanonical(100000000001n)).toBe("100.000000001");
    expect(toCanonical(fromNanos(100000000001n))).toBe("100.000000001");
  });

  it("keeps the sign and pads the fraction", () => {
    expect(formatCanonical(-3140700000000n)).toBe("-3140.700000000");
    expect(formatCanonical(0n)).toBe("0.000000000");
  });

  it("keeps precision a double would lose", () => {
    expect(formatCanonical(9007199254740993n)).toBe("9007199.254740993");
    expect(9007199254740993n > BigInt(Number.MAX_SAFE_INTEGER)).toBe(true);
  });
});

describe("arithmetic", () => {
  it("adds on bigint nanos without binary float", () => {
    const sum = add(parseDecimal("0.100000000"), parseDecimal("0.200000000"));
    expect(toCanonical(sum)).toBe("0.300000000");
  });

  it("subtracts, negates and compares on nanos only", () => {
    const left = parseDecimal("0.300000000");
    const right = parseDecimal("0.100000000");
    expect(toCanonical(sub(left, right))).toBe("0.200000000");
    expect(toCanonical(neg(parseDecimal("272.550000000")))).toBe("-272.550000000");
    expect(compare(right, left)).toBe(-1);
    expect(compare(left, left)).toBe(0);
    expect(compare(left, right)).toBe(1);
    expect(compare(neg(left), right)).toBe(-1);
  });
});

describe("formatDisplay", () => {
  it("groups integer digits from nanos or a canonical string", () => {
    expect(formatDisplay(fromNanos(1284200000000n))).toBe("1 284,200000000");
    expect(formatDisplay("1284.200000000")).toBe("1 284,200000000");
    expect(formatDisplay(parseDecimal("-3140.700000000"))).toBe("-3 140,700000000");
  });
});

describe("exports", () => {
  it("does not export Number coercion helpers", () => {
    const names = Object.keys(decimal);
    expect(names).not.toContain("toNumber");
    expect(names).not.toContain("fromNumber");
    expect(names).not.toContain("parseFloat");
    expect(names).not.toContain("parseInt");
    expect(names).not.toContain("toNumberUnsafe");
  });
});
