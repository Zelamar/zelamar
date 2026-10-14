<#
.SYNOPSIS
  Prints single-use invite codes for the production service.

.DESCRIPTION
  The production INVITE_KEY is stored on this machine only, sealed with
  Windows DPAPI for the current user, at
  %LOCALAPPDATA%\Zelamar\secrets\invite-key.dpapi. It is unsealed in memory
  for the duration of this script and never written in the clear.

  Keep a note of the serials handed out: each serial yields one code.

.EXAMPLE
  .\scripts\prod-invites.ps1 -First 1000 -Count 5
#>
param(
  [Parameter(Mandatory)][uint32]$First,
  [uint32]$Count = 1
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security

$path = Join-Path $env:LOCALAPPDATA 'Zelamar\secrets\invite-key.dpapi'
$sealed = [IO.File]::ReadAllBytes($path)
$context = [Text.Encoding]::ASCII.GetBytes('zelamar/invite-key/v0')
$plain = [System.Security.Cryptography.ProtectedData]::Unprotect($sealed, $context, 'CurrentUser')
try {
  $env:INVITE_KEY = [Text.Encoding]::ASCII.GetString($plain)
  node (Join-Path $PSScriptRoot 'invites.mjs') $First $Count
} finally {
  Remove-Item Env:INVITE_KEY -ErrorAction SilentlyContinue
  [Array]::Clear($plain, 0, $plain.Length)
}
