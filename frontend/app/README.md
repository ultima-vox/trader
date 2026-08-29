# @vox/frontend-foundation

Issue **#18** frontend foundation: typed application infrastructure that consumes the
generated `@vox/api-client`. This is not #30 operator workspaces.

Do **not** edit `frontend/api-client/src`. That tree is generated from
`docs/api/openapi.json`. Change the Rust contracts, regenerate, or the API-contract CI
fails. Resolve the client through the `@vox/api-client` alias (see `tsconfig.json` /
`vitest.config.ts`), never through a published npm package.

## Test

From the repository root:

```bash
npm --prefix frontend/app ci
npm --prefix frontend/app test
npm --prefix frontend/app run typecheck
npx --prefix frontend/app playwright install chromium
npm --prefix frontend/app run test:e2e
```

## Browser rules

- No secrets in URL, `localStorage`, `sessionStorage`, IndexedDB, logs or telemetry.
  Connections are addressed by `OpaqueRef` only.
- No provider calls from the browser. The UI talks to the Vox application API, never to
  T-Invest / `invest-public-api` / `api-invest.tinkoff.ru`.
- Workspace placement is `--vox-grid-col-start` / `--vox-grid-row-start` /
  `--vox-grid-col-span` / `--vox-grid-row-span` (CSS Grid 1-indexed). Persisted
  `col,row,colSpan,rowSpan` are written onto the widget so reload restores geometry.
