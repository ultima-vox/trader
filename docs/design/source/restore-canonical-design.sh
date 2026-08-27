#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC_DIR="$ROOT/docs/design/source"
OUT="$SRC_DIR/Vox-Trader-Design-System-canonical.html"
TMP_XZ="$(mktemp)"
trap 'rm -f "$TMP_XZ"' EXIT

cat   "$SRC_DIR/canonical.xz.b64.part00"   "$SRC_DIR/canonical.xz.b64.part01"   "$SRC_DIR/canonical.xz.b64.part02"   | base64 --decode > "$TMP_XZ"

printf '%s  %s\n'   '928a31ea7d3e1d41f421a4534f3dcc34819b76d7753b1a1f7826352cd3c0832c'   "$TMP_XZ" | sha256sum --check -

xz --decompress --stdout "$TMP_XZ" > "$OUT"

printf '%s  %s\n'   '5da71028760066f8781af367dc42daa1c65a586e544315947fedf71d8a473196'   "$OUT" | sha256sum --check -

echo "Restored canonical design source: $OUT"
