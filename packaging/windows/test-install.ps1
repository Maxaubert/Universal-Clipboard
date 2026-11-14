<#
.SYNOPSIS
  Install and uninstall test for the Windows packages (plan 8.5). CI ONLY: it installs for
  the current user, writes the real Run value, adds a firewall rule (needs an administrator)
  and installs an MSIX with a throwaway certificate. Never run it on a computer someone uses.

.EXAMPLE
  pwsh packaging/windows/test-install.ps1 -Setup dist\UniversalClipboard-0.1.0-windows-x64-setup.exe -Msix dist\UniversalClipboard-0.1.0-windows-x64.msix
#>
param(
  [Parameter(Mandatory = $true)] [string]$Setup,
  [string]$Msix
)
$ErrorActionPreference = "Stop"
if (-not $env:CI) { throw "CI only: this test changes the machine (see the header)." }
$Setup = (Resolve-Path $Setup).Path
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

# --- 1. Silent install.
if ((Run $Setup "/S") -ne 0) { Fail "install exit code" }
foreach ($f in "uc-agent.exe", "uc.exe", "Universal Clipboard.exe", "Uninstall.exe", "cli-name.txt") {
  if (-not (Test-Path (Join-Path $dir $f))) { Fail "$f missing" }
}
if ((Get-Content (Join-Path $dir "cli-name.txt")) -ne "uc") { Fail "cli name should be uc" }
if (-not (Test-Path $uninstKey)) { Fail "uninstall entry missing" }
if ((UserPath) -notlike "*$dir*") { Fail "install folder not on the user PATH" }
& (Join-Path $dir "uc.exe") --version
if ($LASTEXITCODE -ne 0) { Fail "uc --version" }

# --- 2. The real app on this runner: start the agent through uc, turn on start at login.
$uc = Join-Path $dir "uc.exe"
& $uc status --json | Out-Host
if ($LASTEXITCODE -ne 0) { Fail "uc status" }
& $uc settings set start-at-login on | Out-Host
if (-not (Get-ItemProperty $runKey -Name UniversalClipboard -ErrorAction SilentlyContinue)) { Fail "Run value missing" }
if (-not (Test-Path (Join-Path $state "state.json"))) { Fail "state.json missing" }

# --- 3. Firewall helper (the runner is elevated, so no UAC prompt).
$agent = Join-Path $dir "uc-agent.exe"
if ((Run $agent "--firewall add --exe `"$agent`"") -ne 0) { Fail "firewall add" }
$rule = netsh advfirewall firewall show rule name="Universal Clipboard" verbose | Out-String
if ($rule -notmatch "Private" -or $rule -match "Public" -or $rule -notmatch "LocalSubnet") { Fail "firewall rule scope: $rule" }

# --- 4. An update over the top (/UPDATE) keeps the state.
& $uc --version | Out-Host
$null = Run $agent "--stop"
if ((Run $Setup "/S /UPDATE /D=$dir") -ne 0) { Fail "update install" }
if (-not (Test-Path (Join-Path $state "state.json"))) { Fail "an update removed the state" }

# --- 5. Uninstall (silent, also remove the firewall rule).
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

# --- 6. Name rule: another "uc" on PATH, so ours is "uclip".
$fake = Join-Path $env:RUNNER_TEMP "fake-uc"
New-Item -ItemType Directory -Force $fake | Out-Null
Set-Content (Join-Path $fake "uc.cmd") "@echo other"
$env:PATH = "$fake;$env:PATH"
if ((Run $Setup "/S") -ne 0) { Fail "install 2" }
if ((Get-Content (Join-Path $dir "cli-name.txt")) -ne "uclip") { Fail "cli name should be uclip" }
if (-not (Test-Path (Join-Path $dir "uclip.exe")) -or (Test-Path (Join-Path $dir "uc.exe"))) { Fail "uclip.exe" }
if ((Run (Join-Path $dir "Uninstall.exe") "/S _?=$dir") -ne 0) { Fail "uninstall 2" }
Write-Host "NSIS install/uninstall test: PASS"

# --- 7. MSIX: sign with a throwaway certificate, install, run the alias, remove.
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
