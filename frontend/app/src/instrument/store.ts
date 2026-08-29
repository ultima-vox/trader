import type { InstrumentContextListener, InstrumentRef, WidgetInstrumentMode } from "./types";
import { freezeInstrumentRef } from "./types";

export class InstrumentContextStore {
  private current: InstrumentRef | null = null;
  private readonly pinned = new Map<string, InstrumentRef>();
  private readonly listeners = new Set<InstrumentContextListener>();

  global(): InstrumentRef | null {
    return this.current;
  }

  setGlobal(next: InstrumentRef | null): void {
    this.current = next === null ? null : freezeInstrumentRef(next);
    this.emit();
  }

  widgetMode(widgetId: string): WidgetInstrumentMode {
    return this.pinned.has(widgetId) ? "PINNED" : "LINKED";
  }

  pin(widgetId: string, instrument: InstrumentRef): void {
    this.pinned.set(widgetId, freezeInstrumentRef(instrument));
    this.emit();
  }

  unlink(widgetId: string): void {
    this.pinned.delete(widgetId);
    this.emit();
  }

  resolve(widgetId: string): InstrumentRef | null {
    return this.pinned.get(widgetId) ?? this.current;
  }

  subscribe(listener: InstrumentContextListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private emit(): void {
    for (const listener of [...this.listeners]) {
      listener();
    }
  }
}
