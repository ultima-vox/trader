import { append, el } from "./dom";

export type DeferredOptions = {
  title: string;
  body: string;
  owner?: string;
  actions?: HTMLElement[];
};

/** Canonical deferred region: named, disabled, never an error. */
export function createDeferred(options: DeferredOptions): HTMLElement {
  const root = el("div", "vox-deferred");
  const head = el("div", "vox-deferred__head");
  append(head, el("span", "vox-deferred__title", options.title));
  const owner = options.owner;
  if (owner !== undefined && owner !== "") {
    append(head, el("span", "vox-dep", owner));
  }
  append(root, head, el("div", "vox-deferred__body", options.body));
  const actions = options.actions;
  if (actions !== undefined && actions.length > 0) {
    const row = el("div", "vox-deferred__actions");
    append(row, ...actions);
    append(root, row);
  }
  return root;
}
