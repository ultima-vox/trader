import { describe, expect, it } from "vitest";
import { createDataState } from "./data-state";

describe("DataState UNKNOWN", () => {
  it("does not render UNKNOWN as a generic failure", () => {
    const view = createDataState({ state: { kind: "UNKNOWN" } });
    document.body.append(view);

    expect(view.querySelector(".is-error")).toBeNull();
    expect(view.classList.contains("is-error")).toBe(false);
    expect(view.querySelector(".vox-badge--unknown")).not.toBeNull();
    expect(view.textContent).toContain("UNKNOWN");
    expect(view.textContent).toContain("Неизвестно");

    const title = view.querySelector(".vox-state-note__title");
    expect(title).not.toBeNull();
    expect(title?.textContent?.trim()).not.toBe("Ошибка");
    expect(title?.textContent).not.toMatch(/^Ошибка$/);

    view.remove();
  });

  it("renders ERROR with title Ошибка and no unknown badge", () => {
    const view = createDataState({
      state: {
        kind: "ERROR",
        error: {
          code: "INTERNAL",
          message: "сбой",
          correlation_id: "corr-9",
          category: "INTERNAL",
          retryable: false,
        },
      },
    });
    document.body.append(view);
    expect(view.querySelector(".vox-state-note__title")?.textContent).toBe("Ошибка");
    expect(view.querySelector(".vox-badge--unknown")).toBeNull();
    expect(view.textContent).toContain("INTERNAL");
    expect(view.textContent).toContain("corr-9");
    view.remove();
  });
});
