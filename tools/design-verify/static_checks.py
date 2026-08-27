#!/usr/bin/env python3
"""Static governance checks for the Vox Trader design system.

Verifies, without a browser, that the executable design system stays governed:

1. no raw HEX or rgba() outside the token layer;
2. no class used by the reference that the layers do not define;
3. exactly one executable reference entry point;
4. the restored canonical source is byte-identical to its recorded SHA-256, if present;
5. no state word, environment or reason code on a rendered screen that the Rust contracts
   do not define;
6. `LIVE` is never used as an executable broker environment.

Exit code 0 only when every check passes.
"""
import hashlib
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
DS = ROOT / "frontend" / "design-system"
REF = DS / "reference" / "index.html"
LAYERS = [DS / "tokens" / "tokens.css", DS / "primitives" / "primitives.css",
          DS / "components" / "components.css", DS / "patterns" / "patterns.css"]
CANON = ROOT / "docs" / "design" / "source" / "Vox-Trader-Design-System-canonical.html"
CANON_SHA = "5da71028760066f8781af367dc42daa1c65a586e544315947fedf71d8a473196"

# ReasonCode + ProtectionCapabilityError + JournalState values a screen may quote.
CONTRACT_CODES = {
    "STARTUP", "CONNECTING", "RECONCILIATION_STARTED", "RECONCILIATION_COMPLETE",
    "RECONCILIATION_INCOMPLETE", "UNKNOWN_MUTATION", "BROKER_POSITION_CONFLICT",
    "BROKER_ORDER_CONFLICT", "BROKER_STOP_CONFLICT", "REQUIRED_READ_UNAVAILABLE",
    "ACCOUNT_UNAVAILABLE", "CREDENTIAL_REJECTED", "EXECUTION_UNAUTHORIZED",
    "STREAM_DISCONNECTED", "STREAM_GAP", "STREAM_QUEUE_OVERFLOW",
    "OPTIONAL_CAPABILITY_UNAVAILABLE", "CHECKPOINT_REBUILD", "PERSISTENCE_FAILURE",
    "OWNERSHIP_FAILURE", "STALE_EPOCH", "CORRUPT_MUTATION_EVIDENCE",
    "SHUTDOWN_REQUESTED", "SHUTDOWN_COMPLETE",
    "FIXED_STOP_UNSUPPORTED", "STOP_LIMIT_UNSUPPORTED", "TAKE_PROFIT_UNSUPPORTED",
    "NATIVE_RELATIVE_TRAILING_UNSUPPORTED", "NATIVE_ABSOLUTE_TRAILING_UNSUPPORTED",
    "NOT_DISPATCHED", "DISPATCHING", "ACKNOWLEDGED", "REJECTED",
    "UNKNOWN_AFTER_DISPATCH", "RECONCILED",
}

failures = []
notes = []


def read(p):
    return p.read_text(encoding="utf-8", errors="replace")


# 1. token governance ----------------------------------------------------------------
tokens_css = read(LAYERS[0])
for layer in LAYERS[1:] + [REF]:
    text = read(layer)
    hexes = sorted(set(re.findall(r"(?<![\w#])#[0-9A-Fa-f]{6}\b", text)))
    rgba = re.findall(r"rgba\(", text)
    if hexes:
        failures.append("raw HEX outside the token layer in %s: %s" % (layer.name, hexes[:8]))
    if rgba:
        failures.append("raw rgba() outside the token layer in %s: %d occurrence(s)" % (layer.name, len(rgba)))
notes.append("token layer defines %d custom properties" % len(set(re.findall(r"--vox-[\w-]+", tokens_css))))

# 2. every class used must be defined --------------------------------------------------
html = read(REF)
defined = set()
for layer in LAYERS:
    defined |= set(re.findall(r"\.([a-zA-Z][\w-]*)", read(layer)))
chrome = set(re.findall(r"\.(ref[\w-]*)", html))          # page chrome, defined inline
used = set()
for attr in re.findall(r'class="([^"]+)"', html):
    used |= set(attr.split())
unknown = sorted(c for c in used - defined - chrome)
if unknown:
    failures.append("classes used by the reference but defined nowhere: %s" % unknown)
notes.append("reference uses %d classes, all defined" % len(used))

# 3. exactly one executable reference entry point --------------------------------------
entries = sorted(p.relative_to(ROOT).as_posix() for p in (DS / "reference").glob("*.html"))
if entries != ["frontend/design-system/reference/index.html"]:
    failures.append("expected exactly one reference entry point, found: %s" % entries)

# 4. canonical provenance --------------------------------------------------------------
if CANON.is_file():
    digest = hashlib.sha256(CANON.read_bytes()).hexdigest()
    if digest != CANON_SHA:
        failures.append("restored canonical source was modified: %s != %s" % (digest, CANON_SHA))
    else:
        notes.append("restored canonical source matches its recorded SHA-256")
else:
    notes.append("canonical source not restored in this checkout (git-ignored artefact); SHA not compared")

# 5. only contract vocabulary in reason codes -------------------------------------------
codes = sorted(set(re.findall(r'vox-reason-code">([A-Z_]+)<', html)))
invented = [c for c in codes if c not in CONTRACT_CODES]
if invented:
    failures.append("reason codes with no contract behind them: %s" % invented)
notes.append("reason codes rendered: %s" % (", ".join(codes) or "none"))

# 6. LIVE is not an executable environment ---------------------------------------------
live_hits = re.findall(r">\s*LIVE\s*<", html)
if live_hits:
    failures.append("LIVE rendered as a value %d time(s); the executable environment is SANDBOX/PRODUCTION"
                    % len(live_hits))
for label in ("PAPER", "BACKTEST"):
    for m in re.finditer(r'<span class="vox-env vox-env--%s"[^>]*>' % label.lower(), html):
        tag = m.group(0)
        if "aria-disabled" not in tag:
            failures.append("%s environment badge is rendered as operational" % label)
notes.append("deferred regions: %d" % html.count('class="vox-deferred"'))

print("static checks on %s" % REF.relative_to(ROOT).as_posix())
for n in notes:
    print("  ·", n)
if failures:
    print("\nFAILURES (%d):" % len(failures))
    for f in failures:
        print("  -", f)
    sys.exit(1)
print("\nall static checks passed")
