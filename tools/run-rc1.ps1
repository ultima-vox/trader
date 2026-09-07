[CmdletBinding()]
param(
    [ValidatePattern('^(127\.0\.0\.1|\[::1\]):[0-9]{1,5}$')]
    [string]$Bind = "127.0.0.1:8080",

    [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA "VoxTrader\rc1"),

    [switch]$SkipFrontendBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Require-Command {
    param([Parameter(Mandatory)][string]$Name)

    if ($null -eq (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

function Require-EnvironmentValue {
    param([Parameter(Mandatory)][string]$Name)

    $item = Get-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
    if ($null -eq $item -or [string]::IsNullOrWhiteSpace($item.Value)) {
        throw "$Name is required. Export it in this PowerShell session; do not put it in source."
    }
    return $item.Value
}

Require-Command "cargo"
Require-Command "node"
Require-Command "npm"

$bootstrapCredential = Require-EnvironmentValue "VOX_BOOTSTRAP_CREDENTIAL"
if ([Text.Encoding]::UTF8.GetByteCount($bootstrapCredential) -lt 32) {
    throw "VOX_BOOTSTRAP_CREDENTIAL must contain at least 32 UTF-8 bytes."
}

$activeKeyVersion = if ([string]::IsNullOrWhiteSpace($env:VOX_KEK_ACTIVE_VERSION)) {
    "1"
} else {
    $env:VOX_KEK_ACTIVE_VERSION
}
if ($activeKeyVersion -notmatch '^[1-9][0-9]*$') {
    throw "VOX_KEK_ACTIVE_VERSION must be a positive integer."
}
$activeKeyName = "VOX_KEK_HEX_V$activeKeyVersion"
$activeKey = Require-EnvironmentValue $activeKeyName
if ($activeKey -notmatch '^[0-9a-fA-F]{64}$') {
    throw "$activeKeyName must contain exactly 32 bytes encoded as 64 hexadecimal characters."
}

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$frontendRoot = Join-Path $repoRoot "frontend\app"
$frontendDist = Join-Path $frontendRoot "dist"
$persistentRoot = [IO.Path]::GetFullPath($DataDirectory)
$runtimeRoot = Join-Path $persistentRoot "runtime"

New-Item -ItemType Directory -Force -Path $persistentRoot, $runtimeRoot | Out-Null

if (-not $SkipFrontendBuild) {
    Push-Location $frontendRoot
    try {
        & npm ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE." }
        & npm run build
        if ($LASTEXITCODE -ne 0) { throw "frontend build failed with exit code $LASTEXITCODE." }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath (Join-Path $frontendDist "index.html") -PathType Leaf)) {
    throw "Frontend bundle missing at '$frontendDist'. Run without -SkipFrontendBuild first."
}

$env:VOX_ENV = "sandbox"
$env:VOX_LIVE_MUTATIONS_ENABLED = "false"
$env:VOX_TINVEST_ENABLED = "true"
$env:VOX_API_BIND = $Bind
$env:VOX_PLATFORM_DB = Join-Path $persistentRoot "vox-platform.sqlite3"
$env:VOX_SECRET_DB = Join-Path $persistentRoot "vox-secrets.sqlite3"
$env:VOX_RUNTIME_DB_DIR = $runtimeRoot
$env:VOX_FRONTEND_DIR = $frontendDist
$env:VOX_KEK_ACTIVE_VERSION = $activeKeyVersion
$env:VOX_BOOTSTRAP_USER_ID = if ([string]::IsNullOrWhiteSpace($env:VOX_BOOTSTRAP_USER_ID)) {
    "user:00000000-0000-4000-8000-000000000001"
} else {
    $env:VOX_BOOTSTRAP_USER_ID
}
$env:VOX_BOOTSTRAP_EXPIRES_UNIX_MS = if ([string]::IsNullOrWhiteSpace($env:VOX_BOOTSTRAP_EXPIRES_UNIX_MS)) {
    [DateTimeOffset]::UtcNow.AddHours(8).ToUnixTimeMilliseconds().ToString()
} else {
    $env:VOX_BOOTSTRAP_EXPIRES_UNIX_MS
}
$env:VOX_SESSION_COOKIE_SECURE = "false"

$url = "http://$Bind/"
Write-Host "Vox Trader RC1 data: $persistentRoot"
Write-Host "Vox Trader RC1 URL:  $url"
Write-Host "Stop with Ctrl+C. Restart with same KEK and DataDirectory."

Push-Location $repoRoot
try {
    & cargo run --locked -p vox-core --bin vox-server
    if ($LASTEXITCODE -ne 0) { throw "vox-server failed with exit code $LASTEXITCODE." }
} finally {
    Pop-Location
}
