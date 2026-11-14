# Cross-OS end-to-end runner (plan 6.5, milestone M7). STATUS: PENDING.
#
# The lab VMs (win-b, mac-b, lin-gnome, lin-kde; plan 6.0, milestone M0a, decision 9) do not
# exist yet. This script is written so it runs once they do. Until then it stops at the
# host check and says so. Do not point it at a machine that someone works on: on the PC the
# agent runs in tools/winhost, on the Mac on a named pasteboard, and only the VMs use their
# real clipboards (nobody works on them).
#
# What it does, per run:
#   1. Builds uc-agent and uc for each OS (PC: cargo; Mac and VMs: over ssh in ~/uc-e2e).
#   2. Starts one agent per host with profile `dev` (state in a fresh temp folder):
#        PC          winhost uc-agent.exe (private window station, real Windows backend)
#        Mac         a test LaunchAgent via `launchctl bootstrap gui/<uid>` (plan 6.4), with
#                    UCLIP_TEST_PASTEBOARD set and UCLIP_TEST_KEYSTORE=file
#        win-b       ssh, scheduled task in the logged-in session (real clipboard)
#        mac-b       ssh, LaunchAgent with the real pasteboard and Keychain
#        lin-gnome   ssh, `uc-agent --supervise` in the GNOME session
#        lin-kde     ssh, `uc-agent --supervise` in the KDE session
#   3. Pairs every host with the PC through the CLI: `uc pair show --json` on the PC,
#      `uc pair enter <code>` on the other host (real LAN, real mDNS).
#   4. Runs the copy matrix for text and an image (and HTML after M11) in every direction of
#      the desktop cells of the pair table (plan 6.5), with `uc status --json` and the test
#      calls `debug.copy` / `debug.clipboard` (test profiles only).
#   5. Reboot check (VMs only): reboot win-b, mac-b, lin-gnome, lin-kde; each agent must be
#      back and connected within 60 s of login.
#   6. Stops every agent, removes the test LaunchAgent and temp folders.
#
# Usage: pwsh tools/e2e/cross-os.ps1 [-Hosts tools/e2e/hosts.json] [-DryRun]

param(
  [string]$Hosts = (Join-Path $PSScriptRoot 'hosts.json'),
  [switch]$DryRun
)
$ErrorActionPreference = 'Stop'

$Default = @(
  @{ name = 'pc';        kind = 'windows'; how = 'local-winhost' },
  @{ name = 'mac';       kind = 'macos';   how = 'ssh-launchagent'; ssh = 'mac'; pasteboard = 'named' },
  @{ name = 'win-b';     kind = 'windows'; how = 'ssh-task';        ssh = 'win-b' },
  @{ name = 'mac-b';     kind = 'macos';   how = 'ssh-launchagent'; ssh = 'mac-b'; pasteboard = 'general' },
  @{ name = 'lin-gnome'; kind = 'linux';   how = 'ssh-session';     ssh = 'lin-gnome' },
  @{ name = 'lin-kde';   kind = 'linux';   how = 'ssh-session';     ssh = 'lin-kde' }
)
$list = if (Test-Path $Hosts) { Get-Content $Hosts -Raw | ConvertFrom-Json } else { $Default }

function Test-Host($h) {
  if ($h.how -eq 'local-winhost') { return $true }
  $null = & ssh -o BatchMode=yes -o ConnectTimeout=5 $h.ssh 'echo ok' 2>$null
  return $LASTEXITCODE -eq 0
}

$missing = @($list | Where-Object { -not (Test-Host $_) } | ForEach-Object { $_.name })
if ($missing.Count -gt 0) {
  Write-Host "PENDING: these lab hosts are not reachable: $($missing -join ', ')."
  Write-Host 'The lab VMs come from tools/lab (M0a) after decision 9. Nothing was started.'
  exit 3
}
if ($DryRun) { Write-Host 'All hosts answer. Dry run: stop here.'; exit 0 }

# The steps below need the lab and were not run yet (2026-11-14).
throw 'Not run yet: the steps after the host check are written in the header and land with the lab (M0a/M10).'
