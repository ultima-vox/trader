import type { RuntimeHealthDto, RuntimeStateDto } from "@vox/api-client";
import { append, el } from "./dom";

export type RuntimeStatusOptions = {
  health: RuntimeHealthDto;
  onOpen?: (health: RuntimeHealthDto) => void;
};

const MODIFIER: Partial<Record<RuntimeStateDto, string>> = {
  READY: "vox-runtime--ready",
  RECONCILING: "vox-runtime--reconciling",
  DEGRADED: "vox-runtime--degraded",
  HALTED: "vox-runtime--halted",
};

export function createRuntimeStatus(options: RuntimeStatusOptions): HTMLElement {
  const health = options.health;
  const chip = el("button", "vox-runtime");
  chip.type = "button";
  const modifier = MODIFIER[health.state];
  if (modifier !== undefined) chip.classList.add(modifier);
  chip.setAttribute("aria-label", `Рантайм ${health.state}`);
  append(chip, el("span", "vox-dot"), el("span", "vox-runtime__label", health.state));

  const popover = el("div", "vox-popover");
  popover.hidden = true;
  popover.style.position = "absolute";
  popover.style.top = "100%";
  popover.style.left = "0";
  popover.style.zIndex = "4";
  popover.style.marginTop = "4px";
  popover.style.minWidth = "240px";
  popover.style.padding = "8px";
  append(popover, el("div", "vox-text--dense", health.reason), el("div", "vox-reason-code", health.reason_code));
  if (!health.execution_authorized) {
    append(popover, el("div", "vox-text--dense", "исполнение Vox выключено"));
  }
  if (!health.new_exposure_allowed) {
    append(popover, el("div", "vox-text--dense", "новая экспозиция запрещена"));
  }

  const root = el("div");
  root.style.position = "relative";
  append(root, chip, popover);

  chip.addEventListener("click", (event) => {
    event.stopPropagation();
    popover.hidden = !popover.hidden;
    options.onOpen?.(health);
  });
  document.addEventListener("click", (event) => {
    if (!root.contains(event.target as Node)) popover.hidden = true;
  });
  return root;
}
