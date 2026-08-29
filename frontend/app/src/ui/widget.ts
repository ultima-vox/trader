import type { InstrumentRef, WidgetInstrumentMode } from "../instrument";
import type { DataStateKind } from "../state";
import { append, el, setClass } from "./dom";
import { createInstrumentContextIndicator, instrumentModeLabel } from "./instrument-context-indicator";

export type WidgetState = "active" | "stale" | "degraded" | "error" | "pinned";

export type WidgetOptions = {
  title: string;
  instrument: InstrumentRef;
  mode: WidgetInstrumentMode;
  body?: HTMLElement;
  footer?: HTMLElement;
  tools?: HTMLElement[];
  states?: WidgetState[];
  dataKind?: DataStateKind;
};

export function createWidget(options: WidgetOptions): HTMLElement {
  const ticker = options.instrument.ticker === "" ? "UNKNOWN" : options.instrument.ticker;

  const root = el("article", "vox-widget");
  root.style.position = "relative";
  root.setAttribute("data-ticker", ticker);
  root.setAttribute("data-binding", options.mode);
  applyStates(root, options.states ?? [], options.dataKind);

  const header = el("div", "vox-widget__header");
  const title = el("span", "vox-widget__title");
  const context = el("span", "vox-widget__context", `${ticker} · ${instrumentModeLabel(options.mode)}`);
  if (options.mode === "LINKED") context.classList.add("vox-widget__context--instrument");
  append(title, document.createTextNode(options.title), context);

  const tools = el("span", "vox-widget__tools");
  append(
    tools,
    createInstrumentContextIndicator({ instrument: options.instrument, mode: options.mode }),
  );
  if (options.tools !== undefined) append(tools, ...options.tools);
  append(header, title, tools);

  const body = el("div", "vox-widget__body");
  if (options.body !== undefined) append(body, options.body);

  append(root, header, body);
  if (options.footer !== undefined) {
    const footer = el("div", "vox-widget__footer");
    append(footer, options.footer);
    append(root, footer);
  }
  append(root, el("span", "vox-widget__resize"));

  root.addEventListener("pointerdown", () => {
    root.classList.add("is-active");
  });
  return root;
}

function applyStates(root: HTMLElement, states: WidgetState[], dataKind?: DataStateKind): void {
  setClass(root, "is-active", states.includes("active"));
  setClass(root, "is-stale", states.includes("stale") || dataKind === "STALE");
  setClass(root, "is-degraded", states.includes("degraded") || dataKind === "DEGRADED");
  setClass(
    root,
    "is-error",
    states.includes("error") || dataKind === "ERROR" || dataKind === "PERMISSION_DENIED",
  );
  setClass(root, "is-pinned", states.includes("pinned"));
}
