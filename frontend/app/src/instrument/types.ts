import type { InstrumentIdentityDto } from "@vox/api-client";

export type InstrumentRef = Readonly<
  Pick<InstrumentIdentityDto, "provider" | "uid" | "ticker" | "class_code">
>;

export type WidgetInstrumentMode = "LINKED" | "PINNED";

export type InstrumentContext = InstrumentRef;

export type InstrumentContextListener = () => void;

export function instrumentRefFromIdentity(
  identity: InstrumentIdentityDto,
): InstrumentRef {
  return freezeInstrumentRef(identity);
}

export function instrumentIdentityKey(ref: Pick<InstrumentRef, "provider" | "uid">): string {
  return `${ref.provider}\u001f${ref.uid}`;
}

export function sameInstrumentIdentity(
  a: Pick<InstrumentRef, "provider" | "uid">,
  b: Pick<InstrumentRef, "provider" | "uid">,
): boolean {
  return a.provider === b.provider && a.uid === b.uid;
}

export function freezeInstrumentRef(
  ref: Pick<InstrumentIdentityDto, "provider" | "uid" | "ticker" | "class_code">,
): InstrumentRef {
  return Object.freeze({
    provider: ref.provider,
    uid: ref.uid,
    ticker: ref.ticker,
    class_code: ref.class_code,
  });
}
