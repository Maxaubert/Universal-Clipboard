<#
.SYNOPSIS
  Ends our programs that run from one install folder, so the installer can replace them.
  Called by installer.nsi (install and uninstall), packed into the setup exe.

.DESCRIPTION
  Bug 2026-11-14: the installer stopped the agent first and closed the window second. The open
  window saw the agent go away and started it again, so uc-agent.exe was in use when the files
  were copied, and NSIS skipped it. So the order is:

    -Phase window   close the window ("Universal Clipboard.exe"), wait until it is gone
                    (5 s, then end it). From now on nothing restarts the agent.
    -Phase rest     stop the agent (-StopAgent: "uc-agent --stop", repeated while it runs),
                    wait up to 15 s until no uc-agent.exe, uc.exe, uclip.exe or window from
                    this folder runs, then end what is left (the last resort, like
                    "taskkill /F") and wait 5 s more.

  Only processes whose program file is inside -Dir count, so another install (or a test
  install in another folder) is never touched.

  Exit codes: 0 nothing of ours runs from the folder, 10 something still runs. Any other code
  means this script did not run (for example a blocked PowerShell); the installer then falls
  back and its own file check decides.
#>
param(
  [Parameter(Mandatory = $true)] [string]$Dir,
  [Parameter(Mandatory = $true)] [ValidateSet("window", "rest")] [string]$Phase,
  [switch]$StopAgent
)
$ErrorActionPreference = "Continue"
$prefix = [IO.Path]::GetFullPath($Dir).TrimEnd('\') + '\'
$window = "Universal Clipboard.exe"
$ours = @("uc-agent.exe", "uc.exe", "uclip.exe", $window)

# Our processes that run from the folder. Win32_Process gives the path of 64-bit processes
# also to a 32-bit PowerShell (Get-Process .Path does not). Callers wrap it in @(): in
# PowerShell 5.1 one CIM object has no .Count.
function FromDir([string[]]$names) {
  @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
      $_.ExecutablePath -and $names -contains $_.Name -and
      $_.ExecutablePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    })
}

function WaitGone([string[]]$names, [int]$ms) {
  $t = [Diagnostics.Stopwatch]::StartNew()
  while ($t.ElapsedMilliseconds -lt $ms) {
    if (@(FromDir $names).Count -eq 0) { return $true }
    Start-Sleep -Milliseconds 200
  }
  return @(FromDir $names).Count -eq 0
}

function EndAll([string[]]$names) {
  foreach ($p in FromDir $names) {
    Write-Output "Ending $($p.Name) ($($p.ProcessId))"
    Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
  }
}

if ($Phase -eq "window") {
  $asked = $false
  foreach ($p in FromDir @($window)) {
    $proc = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue
    if ($proc -and $proc.CloseMainWindow()) { $asked = $true }
  }
  # A window that was asked to close gets 5 s; one with no window to ask is ended at once.
  if (-not $asked -or -not (WaitGone @($window) 5000)) { EndAll @($window) }
  if (WaitGone @($window) 5000) { exit 0 }
  Write-Output "The window still runs."
  exit 10
}

$agent = Join-Path $prefix "uc-agent.exe"
$t = [Diagnostics.Stopwatch]::StartNew()
while ($t.Elapsed.TotalSeconds -lt 15 -and @(FromDir $ours).Count -gt 0) {
  if ($StopAgent -and @(FromDir @("uc-agent.exe")).Count -gt 0 -and (Test-Path $agent)) {
    # The agent's own clean stop (IPC "quit"); it waits up to 10 s itself.
    Start-Process -FilePath $agent -ArgumentList "--stop" -Wait -WindowStyle Hidden -ErrorAction SilentlyContinue
  } else {
    Start-Sleep -Milliseconds 250
  }
}
EndAll $ours
if (WaitGone $ours 5000) { exit 0 }
Write-Output ("Still running: " + ((FromDir $ours | ForEach-Object Name) -join ", "))
exit 10
