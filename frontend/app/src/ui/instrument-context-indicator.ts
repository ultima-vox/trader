import type { InstrumentRef, WidgetInstrumentMode } from "../instrument";
import { el } from "./dom";

export type InstrumentContextIndicatorOptions = {
  instrument: InstrumentRef;
  mode: WidgetInstrumentMode;
};

export function instrumentModeLabel(mode: WidgetInstrumentMode): string {
  return mode === "PINNED" ? "закреплён" : "связан";
}

export function createInstrumentContextIndicator(
  options: InstrumentContextIndicatorOptions,
): HTMLElement {
  const ticker = options.instrument.ticker === "" ? "UNKNOWN" : options.instrument.ticker;
  const node = el("span", "vox-context-link", `${ticker} · ${instrumentModeLabel(options.mode)}`);
  node.classList.add(options.mode === "PINNED" ? "is-pinned" : "is-linked");
  node.title = instrumentModeLabel(options.mode);
  return node;
}
