import type { ApiError } from "@vox/api-client";
import type { DataState } from "../state";
import { append, el } from "./dom";

export type DataStateOptions<T = unknown> = {
  state: DataState<T>;
  children?: HTMLElement | HTMLElement[];
};

export function createDataState<T>(options: DataStateOptions<T>): HTMLElement {
  const kind = options.state.kind;
  const children = flatten(options.children);

  if (kind === "READY") {
    const ready = el("div");
    append(ready, ...children);
    return ready;
  }

  if (kind === "LOADING") {
    const loading = el("div", "vox-stack vox-gap-2");
    append(loading, el("div", "vox-skeleton"), el("div", "vox-skeleton"), el("div", "vox-skeleton"));
    return loading;
  }

  if (kind === "STALE") {
    const wrap = el("div");
    const bar = el("div", "vox-stale-bar");
    const age = options.state.age_ms;
    const ageText = `возраст ${formatAge(age)}`;
    append(bar, document.createTextNode(`Показаны последние известные значения · ${ageText}`));
    append(wrap, bar, ...children);
    return wrap;
  }

  if (kind === "UNKNOWN") {
    const note = el("div", "vox-state-note");
    const title = el("span", "vox-state-note__title");
    const badge = el("span", "vox-badge vox-badge--unknown");
    append(badge, el("span", "vox-dot"), document.createTextNode("UNKNOWN"));
    append(title, badge, document.createTextNode(" Неизвестно"));
    append(
      note,
      title,
      el(
        "span",
        undefined,
        "Состояние неизвестно. Это не отказ и не ошибка — ответ не получен.",
      ),
      ...diagnostics(options.state.error),
    );
    return note;
  }

  if (kind === "RECONNECTING" || kind === "DEGRADED") {
    const note = el("div", "vox-state-note");
    const title = el("span", "vox-state-note__title");
    const warning = kind === "DEGRADED";
    const badge = el("span", warning ? "vox-badge vox-badge--warning" : "vox-badge vox-badge--info");
    append(badge, el("span", "vox-dot"), document.createTextNode(kind));
    append(title, badge, document.createTextNode(warning ? " Деградация" : " Переподключение"));
    append(
      note,
      title,
      el(
        "span",
        undefined,
        warning
          ? "Канал деградировал. Последние известные значения остаются на экране."
          : "Идёт переподключение. Это не ошибка.",
      ),
      ...diagnostics(kind === "DEGRADED" ? options.state.error : undefined),
      ...children,
    );
    return note;
  }

  if (kind === "ERROR") {
    return failureNote("Ошибка", options.state.error, "Запрос завершился отказом.");
  }

  if (kind === "PERMISSION_DENIED") {
    return failureNote(
      "Недостаточно прав",
      options.state.error,
      "Нет права на этот просмотр. Скрытие кнопки не является защитой.",
    );
  }

  const note = el("div", "vox-state-note");
  append(
    note,
    el("span", "vox-state-note__title", "Нет данных"),
    el("span", undefined, "Пока нечего показать. Данные появятся, когда контракт их отдаст."),
  );
  return note;
}

function failureNote(title: string, error: ApiError, fallback: string): HTMLElement {
  const note = el("div", "vox-state-note");
  append(
    note,
    el("span", "vox-state-note__title", title),
    el("span", undefined, error.message === "" ? fallback : error.message),
    ...diagnostics(error),
  );
  return note;
}

function diagnostics(error: ApiError | undefined): HTMLElement[] {
  if (error === undefined) return [];
  const nodes: HTMLElement[] = [el("span", "vox-reason-code", error.code)];
  if (error.correlation_id !== "") {
    nodes.push(el("span", "vox-reason-code", error.correlation_id));
  }
  return nodes;
}

function flatten(children: HTMLElement | HTMLElement[] | undefined): HTMLElement[] {
  if (children === undefined) return [];
  return Array.isArray(children) ? children : [children];
}

function formatAge(ms: number): string {
  if (ms < 1000) return `${ms} мс`;
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds} с`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} мин`;
  return `${Math.floor(minutes / 60)} ч`;
}
