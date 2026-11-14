<#
.SYNOPSIS
  Install and uninstall test for the Windows packages (plan 8.5). CI ONLY: it installs for
  the current user, writes the real Run value, adds a firewall rule (the installer's /ADDFIREWALL; needs an administrator)
  and installs an MSIX with a throwaway certificate. Never run it on a computer someone uses.

.EXAMPLE
  pwsh packaging/windows/test-install.ps1 -Setup dist\UniversalClipboard-0.1.0-windows-x64-setup.exe -Msix dist\UniversalClipboard-0.1.0-windows-x64.msix
#>
param(
  [Parameter(Mandatory = $true)] [string]$Setup,
  [string]$Msix,
  # The files the setup exe was built from (build.ps1 leaves them in <out>\stage-windows).
  [string]$Stage
)
$ErrorActionPreference = "Stop"
if (-not $env:CI) { throw "CI only: this test changes the machine (see the header)." }
$Setup = (Resolve-Path $Setup).Path
if (-not $Stage) { $Stage = Join-Path (Split-Path $Setup) "stage-windows" }
$Stage = (Resolve-Path $Stage).Path
$dir = Join-Path $env:LOCALAPPDATA "Programs\Universal Clipboard"
$state = Join-Path $env:LOCALAPPDATA "UniversalClipboard"
$uninstKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\UniversalClipboard"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
function Fail($m) { throw "FAIL: $m" }
function UserPath { (Get-ItemProperty HKCU:\Environment -Name Path -ErrorAction SilentlyContinue).Path }
function Run($exe, $argList) {
  $p = Start-Process -FilePath $exe -ArgumentList $argList -Wait -PassThru -NoNewWindow
  return $p.ExitCode
}

# Every program file in the install folder is the new build, byte for byte.
$exes = "uc-agent.exe", "uc.exe", "Universal Clipboard.exe"
function Hash($f) { (Get-FileHash -Algorithm SHA256 $f).Hash }
function AssertNew($why) {
  foreach ($e in $exes) {
    $f = Join-Path $dir $e
    if (-not (Test-Path $f)) { Fail "${why}: $e missing" }
    if ((Hash $f) -ne (Hash (Join-Path $Stage $e))) { Fail "${why}: $e is not the new build" }
  }
}
function OursRunning($name) {
  @(Get-CimInstance Win32_Process | Where-Object { $_.Name -eq $name -and $_.ExecutablePath -like "$dir\*" })
}

function RulePresent {
  netsh advfirewall firewall show rule name="Universal Clipboard" | Out-Null
  return $LASTEXITCODE -eq 0
}
# Start clean: no rule from an earlier run.
netsh advfirewall firewall delete rule name="Universal Clipboard" | Out-Null

# --- 1. Silent install with /ADDFIREWALL: the installer adds the firewall rule (decision
#        2026-11-14; the runner is elevated, so no UAC prompt).
if ((Run $Setup "/S /ADDFIREWALL") -ne 0) { Fail "install exit code" }
foreach ($f in "uc-agent.exe", "uc.exe", "Universal Clipboard.exe", "Uninstall.exe", "cli-name.txt") {
  if (-not (Test-Path (Join-Path $dir $f))) { Fail "$f missing" }
}
if ((Get-Content (Join-Path $dir "cli-name.txt")) -ne "uc") { Fail "cli name should be uc" }
if (-not (Test-Path $uninstKey)) { Fail "uninstall entry missing" }
if ((UserPath) -notlike "*$dir*") { Fail "install folder not on the user PATH" }
& (Join-Path $dir "uc.exe") --version
if ($LASTEXITCODE -ne 0) { Fail "uc --version" }

# --- 2. The firewall rule from the installer: Private, LocalSubnet, never Public.
if (-not (RulePresent)) { Fail "the installer did not add the firewall rule" }
$rule = netsh advfirewall firewall show rule name="Universal Clipboard" verbose | Out-String
if ($rule -notmatch "Private" -or $rule -match "Public" -or $rule -notmatch "LocalSubnet") { Fail "firewall rule scope: $rule" }
$agent = Join-Path $dir "uc-agent.exe"
if ((Run $agent "--firewall check") -ne 0) { Fail "firewall check" }

# --- 3. The real app on this runner: start the agent through uc. Start at login is on from
#        the first run (no setup question), the network accepts incoming (the rule is there).
$uc = Join-Path $dir "uc.exe"
& $uc status --json | Out-Host
if ($LASTEXITCODE -ne 0) { Fail "uc status" }
if (-not (Get-ItemProperty $runKey -Name UniversalClipboard -ErrorAction SilentlyContinue)) { Fail "Run value missing after the first run" }
$fw = & $uc firewall check --json | ConvertFrom-Json
if ($fw.rule -ne "present" -or -not $fw.incoming) { Fail "agent firewall check: $($fw | ConvertTo-Json -Compress)" }
& $uc settings set start-at-login on | Out-Host
if (-not (Get-ItemProperty $runKey -Name UniversalClipboard -ErrorAction SilentlyContinue)) { Fail "Run value missing" }
if (-not (Test-Path (Join-Path $state "state.json"))) { Fail "state.json missing" }
AssertNew "first install"

# --- 4. Over the top while the app runs (bug 2026-11-14: "setup.exe /S" replaced uc.exe and
#        the window but not uc-agent.exe, exit 0, and the old agent kept running). The open
#        window starts the agent again when it goes away; the stand-in does the same: a copy
#        of cmd.exe named "Universal Clipboard.exe" in the install folder that runs
#        "uc status" (which starts the agent, like the window) every second.
#        First make every installed program differ from the new build ("the old version").
$null = Run $agent "--stop"
foreach ($e in "uc-agent.exe", "uc.exe") { [IO.File]::AppendAllText((Join-Path $dir $e), "old") }
Copy-Item (Join-Path $env:WINDIR "System32\cmd.exe") (Join-Path $dir "Universal Clipboard.exe") -Force
$restart = Join-Path $env:RUNNER_TEMP "uc-window-standin.cmd"
Set-Content $restart "@echo off`r`n:loop`r`n`"%UC_TEST_DIR%\uc.exe`" status >nul 2>&1`r`nping -n 2 127.0.0.1 >nul`r`ngoto loop`r`n"
$env:UC_TEST_DIR = $dir
$standin = Start-Process -FilePath (Join-Path $dir "Universal Clipboard.exe") -ArgumentList "/d /c `"$restart`"" -WindowStyle Hidden -PassThru
function WaitAgent($want, $why) {
  $t = [Diagnostics.Stopwatch]::StartNew()
  while ($t.Elapsed.TotalSeconds -lt 20) {
    if (((OursRunning "uc-agent.exe").Count -gt 0) -eq $want) { return }
    Start-Sleep -Milliseconds 250
  }
  Fail $why
}
WaitAgent $true "the stand-in window did not start the agent"
# The race itself: stop the agent, and the stand-in starts it again.
$null = Run $agent "--stop"
WaitAgent $true "the stand-in window did not start the agent again"
$code = Run $Setup "/S /D=$dir"
if ($code -ne 0) { Fail "install over the running app: exit code $code" }
AssertNew "install over the running app"
if (-not $standin.HasExited) { Fail "the installer left the window running" }
if ((OursRunning "uc-agent.exe").Count -gt 0) { Fail "an agent still runs after the install" }
if (-not (Test-Path (Join-Path $state "state.json"))) { Fail "the install removed the state" }

# --- 5. A program file that stays in use fails the install with an exit code, and nothing is
#        replaced (here the test holds uc.exe open; the installer cannot end this process).
[IO.File]::AppendAllText((Join-Path $dir "uc-agent.exe"), "old")
$lock = [IO.File]::Open((Join-Path $dir "uc.exe"), "Open", "ReadWrite", "None")
try {
  $code = Run $Setup "/S /D=$dir"
} finally {
  $lock.Close()
}
if ($code -eq 0) { Fail "a locked uc.exe did not fail the install" }
Write-Host "locked file: install exit code $code (expected non-zero)"
if ((Hash (Join-Path $dir "uc-agent.exe")) -eq (Hash (Join-Path $Stage "uc-agent.exe"))) { Fail "a failed install replaced some files" }
foreach ($e in $exes) { if (-not (Test-Path (Join-Path $dir $e))) { Fail "a failed install removed $e" } }

# --- 6. An update over the top (/UPDATE, as the update helper runs it) keeps the state and
#        replaces every program.
& $uc --version | Out-Host
$null = Run $agent "--stop"
foreach ($e in "uc-agent.exe", "uc.exe") { [IO.File]::AppendAllText((Join-Path $dir $e), "old") }
if ((Run $Setup "/S /UPDATE /D=$dir") -ne 0) { Fail "update install" }
AssertNew "update"
if (-not (Test-Path (Join-Path $state "state.json"))) { Fail "an update removed the state" }
if (-not (RulePresent)) { Fail "an update removed the firewall rule" }

# --- 7. Uninstall (silent, also remove the firewall rule).
& $uc status --json | Out-Null
if ((Run (Join-Path $dir "Uninstall.exe") "/S /REMOVEFIREWALL _?=$dir") -ne 0) { Fail "uninstall exit code" }
Start-Sleep -Seconds 2
# With _?= the uninstaller runs in place (so -Wait works) and cannot delete itself.
foreach ($f in "uc-agent.exe", "uc.exe", "uclip.exe", "Universal Clipboard.exe", "cli-name.txt") {
  if (Test-Path (Join-Path $dir $f)) { Fail "$f left behind" }
}
if (Test-Path $uninstKey) { Fail "uninstall entry left behind" }
if (Get-ItemProperty $runKey -Name UniversalClipboard -ErrorAction SilentlyContinue) { Fail "Run value left behind" }
if (Test-Path $state) { Fail "state folder left behind" }
if ((UserPath) -like "*$dir*") { Fail "PATH entry left behind" }
if (Get-Process uc-agent -ErrorAction SilentlyContinue) { Fail "agent still running" }
netsh advfirewall firewall show rule name="Universal Clipboard" | Out-Null
if ($LASTEXITCODE -eq 0) { Fail "firewall rule left behind" }
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue

# --- 8. Name rule: another "uc" on PATH, so ours is "uclip". Also: a silent install without
#        /ADDFIREWALL adds no rule (it never shows a UAC prompt).
$fake = Join-Path $env:RUNNER_TEMP "fake-uc"
New-Item -ItemType Directory -Force $fake | Out-Null
Set-Content (Join-Path $fake "uc.cmd") "@echo other"
$env:PATH = "$fake;$env:PATH"
if ((Run $Setup "/S") -ne 0) { Fail "install 2" }
if ((Get-Content (Join-Path $dir "cli-name.txt")) -ne "uclip") { Fail "cli name should be uclip" }
if (RulePresent) { Fail "a silent install without /ADDFIREWALL added the firewall rule" }
if (-not (Test-Path (Join-Path $dir "uclip.exe")) -or (Test-Path (Join-Path $dir "uc.exe"))) { Fail "uclip.exe" }
if ((Run (Join-Path $dir "Uninstall.exe") "/S _?=$dir") -ne 0) { Fail "uninstall 2" }
Write-Host "NSIS install/uninstall test: PASS"

# --- 9. MSIX: sign with a throwaway certificate, install, run the alias, remove.
if ($Msix) {
  $Msix = (Resolve-Path $Msix).Path
  $cert = New-SelfSignedCertificate -Type Custom -Subject "CN=UniversalClipboardDev" -KeyUsage DigitalSignature `
    -FriendlyName "uclip CI" -CertStoreLocation "Cert:\CurrentUser\My" `
    -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
  $pfx = Join-Path $env:RUNNER_TEMP "uclip-ci.pfx"
  $pw = ConvertTo-SecureString -String "ci" -Force -AsPlainText
  Export-PfxCertificate -Cert $cert -FilePath $pfx -Password $pw | Out-Null
  Import-PfxCertificate -FilePath $pfx -CertStoreLocation Cert:\LocalMachine\TrustedPeople -Password $pw | Out-Null
  $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
  $signtool = Get-ChildItem $kits -Recurse -Filter signtool.exe | Where-Object { $_.FullName -match '\\x64\\' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  $signed = Join-Path $env:RUNNER_TEMP "signed.msix"
  Copy-Item $Msix $signed
  & $signtool.FullName sign /fd SHA256 /f $pfx /p ci $signed
  if ($LASTEXITCODE -ne 0) { Fail "signtool" }
  Add-AppxPackage -Path $signed
  $pkg = Get-AppxPackage -Name Maxaubert.UniversalClipboard
  if (-not $pkg) { Fail "MSIX not installed" }
  # The window is in the package (never started here).
  if (-not (Test-Path (Join-Path $pkg.InstallLocation "Universal Clipboard.exe"))) { Fail "window missing in the MSIX" }
  if ((Get-Content -Raw (Join-Path $pkg.InstallLocation "AppxManifest.xml")) -notmatch 'Executable="Universal Clipboard.exe"') { Fail "MSIX manifest has no window app" }
  $alias = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps\uclip.exe"
  if (-not (Test-Path $alias)) { Fail "uclip alias missing" }
  & $alias --version
  if ($LASTEXITCODE -ne 0) { Fail "uclip alias does not run" }
  Remove-AppxPackage -Package $pkg.PackageFullName
  if (Get-AppxPackage -Name Maxaubert.UniversalClipboard) { Fail "MSIX still installed" }
  if (Test-Path $alias) { Fail "alias left behind" }
  Write-Host "MSIX install/uninstall test: PASS"
}
