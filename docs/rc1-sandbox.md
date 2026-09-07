# Vox Trader RC1: T-Invest Sandbox

This runbook starts production composition on Windows from clean checkout. Server serves built
frontend and `/api/v1` from same loopback origin. Broker token enters through browser and existing
encrypted connection store; no Rust or TypeScript source edit is needed.

## Prerequisites

- Windows PowerShell 5.1 or PowerShell 7;
- Git;
- Rust toolchain manager (`rustup`); repository selects Rust 1.97.1;
- Node.js 22 LTS with npm;
- T-Invest Sandbox token with access to at least one Sandbox account.

## First start

Clone and enter checkout:

```powershell
git clone https://github.com/ultima-vox/trader.git
Set-Location trader
```

Generate two local operator secrets. Keep KEK in OS/user secret manager. Same
`VOX_KEK_HEX_V1` is required after restart to decrypt stored T-Invest credential. Never commit
either value or place it in `.env`.

```powershell
$env:VOX_KEK_HEX_V1 = [Convert]::ToHexString(
    [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
).ToLowerInvariant()
$env:VOX_BOOTSTRAP_CREDENTIAL = [Convert]::ToHexString(
    [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
).ToLowerInvariant()
```

PowerShell 5.1 lacks `RandomNumberGenerator.GetBytes(Int32)`. Use this equivalent there:

```powershell
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
$kek = New-Object byte[] 32
$bootstrap = New-Object byte[] 32
$rng.GetBytes($kek)
$rng.GetBytes($bootstrap)
$rng.Dispose()
$env:VOX_KEK_HEX_V1 = -join ($kek | ForEach-Object { $_.ToString('x2') })
$env:VOX_BOOTSTRAP_CREDENTIAL = -join ($bootstrap | ForEach-Object { $_.ToString('x2') })
```

Start canonical RC1 path:

```powershell
.\tools\run-rc1.ps1
```

Script runs locked frontend install/build, creates persistent DB paths under
`%LOCALAPPDATA%\VoxTrader\rc1`, then runs `cargo run --locked -p vox-core --bin vox-server`.
Open <http://127.0.0.1:8080/>. Enter value of `VOX_BOOTSTRAP_CREDENTIAL` in **Bootstrap
credential**. Server exchanges it for HttpOnly, SameSite=Strict browser session cookie.

Loopback uses HTTP with `VOX_SESSION_COOKIE_SECURE=false`. Do not expose this RC1 command on LAN
or Internet. Non-loopback bind is rejected by script and server.

## Broker setup in browser

1. Under **T-Invest Sandbox setup**, keep or change connection label.
2. Paste T-Invest Sandbox token into password field. Select **Connect and discover accounts**.
3. Select accessible account returned by T-Invest. Vox creates binding, enables manual Sandbox
   execution authorization, and starts account runtime.
4. In **Server risk**, select **Set risk state NORMAL** and confirm.
5. Search instrument by ticker or name. Select broker result and verify real last quote appears.
6. Enter quantity in lots. For LIMIT, enter exact price. Select BUY or SELL and confirm dispatch.
7. Use **Refresh broker evidence** to inspect order state, positions, portfolio, stop orders,
   mutation ACK/FILL/REJECT/UNKNOWN state, and reconciliation state.
8. After broker fill appears, use **Broker-native protection** with position side, protected lots,
   reference price, and stop-loss trigger. Vox links entry reservation when current receipt has one.
9. To reduce or close, select same instrument and use position close action. Refresh until broker
   position and portfolio reflect result.

T-Invest token is sent once to secure connection API, encrypted at rest with AES-256-GCM envelope
keying, cleared from input, and never returned by API. Do not set `TINVEST_SANDBOX_TOKEN` for normal
application use; that variable belongs to ignored qualification tests.

## Restart and recovery

Stop server with Ctrl+C. Preserve `%LOCALAPPDATA%\VoxTrader\rc1` and re-export same KEK from secret
manager in new PowerShell session. Re-export bootstrap credential, then skip unchanged frontend
build:

```powershell
$env:VOX_KEK_HEX_V1 = '<same-64-hex-character-KEK>'
$env:VOX_BOOTSTRAP_CREDENTIAL = '<operator-bootstrap-secret>'
.\tools\run-rc1.ps1 -SkipFrontendBuild
```

Server reopens platform/secret DBs, restores enabled binding runtimes, and reconciles broker state
before admitting new exposure. Browser may require bootstrap login again. Verify runtime reaches
READY, then refresh broker evidence and compare positions, portfolio, orders, stops, mutations,
and reconciliation results.

Custom persistent location and port:

```powershell
.\tools\run-rc1.ps1 -DataDirectory 'D:\VoxTrader\rc1' -Bind '127.0.0.1:18080'
```

KEK rotation requires old version variables to remain available until stored credentials are
rewrapped. Deleting DB directory resets local application state and loses encrypted broker
credential metadata.

## Local verification without broker credential

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
npm --prefix frontend/app ci --no-audit --no-fund
npm --prefix frontend/app test
npm --prefix frontend/app run typecheck
npm --prefix frontend/app run build
```

Broker discovery, quotes, Sandbox mutations, broker-native protection, and restart comparison need
real user credential and must be reported `GATED_NO_CREDENTIAL` when unavailable.
