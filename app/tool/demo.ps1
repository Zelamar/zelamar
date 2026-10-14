<#
.SYNOPSIS
  Local two-window demo: Alice and Bob chat through a real delivery service
  running on this machine.

.DESCRIPTION
  1. Starts the delivery service (`wrangler dev`) on http://127.0.0.1:8787
     unless one is already running.
  2. Builds the Windows app in release mode (skip with -NoBuild).
  3. Prints two fresh single-use invite codes.
  4. Opens two app windows with separate profiles and stores.

  Needs server/.dev.vars with INVITE_KEY (hex), as for `wrangler dev`.
#>
param(
  [switch]$NoBuild,
  [string[]]$Profiles = @('alice', 'bob')
)

$ErrorActionPreference = 'Stop'
$app = Split-Path -Parent $PSScriptRoot
$server = Join-Path (Split-Path -Parent $app) 'server'
$service = 'http://127.0.0.1:8787'

function Test-Service {
  try { (Invoke-WebRequest "$service/health" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200 }
  catch { $false }
}

if (-not (Test-Service)) {
  Write-Host 'Starting the delivery service...'
  $wrangler = Join-Path $server 'node_modules\.bin\wrangler.cmd'
  Start-Process -FilePath $wrangler -ArgumentList 'dev', '--port', '8787', '--ip', '127.0.0.1' `
    -WorkingDirectory $server -WindowStyle Minimized
  $deadline = (Get-Date).AddSeconds(60)
  while (-not (Test-Service)) {
    if ((Get-Date) -gt $deadline) { throw 'The delivery service did not start.' }
    Start-Sleep -Milliseconds 500
  }
}
Write-Host "Delivery service: $service"

if (-not $NoBuild) {
  Push-Location $app
  try { flutter build windows --release } finally { Pop-Location }
}

$key = (Get-Content (Join-Path $server '.dev.vars') | Where-Object { $_ -match '^INVITE_KEY=' }) `
  -replace '^INVITE_KEY="?([0-9a-f]+)"?$', '$1'
$env:INVITE_KEY = $key
$first = Get-Random -Minimum 100000 -Maximum 2000000000
Write-Host ''
Write-Host 'Invite codes (one per account, single use):'
node (Join-Path $server 'scripts\invites.mjs') $first $Profiles.Count
Write-Host ''

$exe = Join-Path $app 'build\windows\x64\runner\Release\zelamar.exe'
foreach ($profile in $Profiles) {
  Start-Process -FilePath $exe -ArgumentList "--profile=$profile", "--service=$service"
}
Write-Host "Opened: $($Profiles -join ', ')"
