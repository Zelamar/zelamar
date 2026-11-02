<#
.SYNOPSIS
  Makes one invite code for the production service, copies it, and shows
  it: what the desktop shortcut "Zelamar invite" runs.

.DESCRIPTION
  The production INVITE_KEY is sealed with Windows DPAPI for the current
  user at %LOCALAPPDATA%\Zelamar\secrets\invite-key.dpapi and unsealed in
  memory only. Each code uses the next serial from invite-next.txt beside
  it, so no serial is ever handed out twice.
#>
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
Add-Type -AssemblyName System.Windows.Forms

$secrets = Join-Path $env:LOCALAPPDATA 'Zelamar\secrets'
$counter = Join-Path $secrets 'invite-next.txt'
try {
  $serial = if (Test-Path -LiteralPath $counter) {
    [uint32](Get-Content -LiteralPath $counter -Raw).Trim()
  } else {
    2000
  }
  $sealed = [IO.File]::ReadAllBytes((Join-Path $secrets 'invite-key.dpapi'))
  $context = [Text.Encoding]::ASCII.GetBytes('zelamar/invite-key/v0')
  $plain = [System.Security.Cryptography.ProtectedData]::Unprotect($sealed, $context, 'CurrentUser')
  try {
    $env:INVITE_KEY = [Text.Encoding]::ASCII.GetString($plain)
    $line = node (Join-Path $PSScriptRoot 'invites.mjs') $serial 1
  } finally {
    Remove-Item Env:INVITE_KEY -ErrorAction SilentlyContinue
    [Array]::Clear($plain, 0, $plain.Length)
  }
  $code = ($line | Select-Object -Last 1).Trim() -split '\s+' | Select-Object -Last 1
  if ($code -notmatch '^[a-z2-7]{5}(-[a-z2-7]{5}){3}$') {
    throw "unexpected output: $line"
  }
  Set-Content -LiteralPath $counter -Value ($serial + 1) -NoNewline
  Set-Clipboard -Value $code
  [System.Windows.Forms.MessageBox]::Show(
    "Приглашение в Zelamar:`n`n$code`n`nУже скопировано — вставьте его другу в сообщение.`nКаждый код работает один раз.",
    'Zelamar — приглашение',
    'OK',
    'Information'
  ) | Out-Null
} catch {
  [System.Windows.Forms.MessageBox]::Show(
    "Не получилось сделать приглашение.`n`n$($_.Exception.Message)",
    'Zelamar — приглашение',
    'OK',
    'Error'
  ) | Out-Null
}
