# reference/assets

Local assets for the static reference sheet (`../index.html`). No external CDN, no
network calls — the reference must render from `file://` with no connectivity.

| File | Use |
| --- | --- |
| `vox-mark.svg` | Product mark in the top bar / favicon of the reference sheet. |

Asset rules

- Relative paths only (`./assets/…`), never absolute or host-qualified.
- Icons in product code come from a single family (Lucide) behind the `.vox-icon`
  abstraction; the reference sheet draws inline `<svg>` glyphs so it stays offline.
- Fonts are **not** bundled. `--vox-font-ui` / `--vox-font-mono` degrade to system
  stacks (`system-ui`, `ui-monospace`). If the product later ships Inter and
  JetBrains Mono, add the woff2 files here and one `@font-face` block in
  `../../tokens/tokens.css`.
- No screenshots are used as specification. Raster images, if ever added, are
  illustration only.
