<#
.SYNOPSIS
  Build the Windows packages: the per-user NSIS installer and the MSIX (Store build).
  Both are UNSIGNED. Nothing is installed.

.EXAMPLE
  cargo build --release -p uc-agent -p uc-cli
  pwsh packaging/windows/build.ps1 -Version 0.1.0 -Bin target/release -Out dist

.NOTES
  - -Bin must hold uc-agent.exe and uc.exe. "Universal Clipboard.exe" (the window, M8b) is
    packaged too when it is there.
  - The Store signs the MSIX on submit. A sideloaded MSIX needs our own certificate
    (-Publisher must match its subject); CI makes a throwaway one to test install/uninstall.
#>
param(
  [Parameter(Mandatory = $true)] [string]$Version,
  [Parameter(Mandatory = $true)] [string]$Bin,
  [string]$Out = "dist",
  [ValidateSet("x64", "arm64")] [string]$Arch = "x64",
  [string]$Publisher = "CN=UniversalClipboardDev",
  [string]$PublisherName = "Universal Clipboard contributors",
  [switch]$NoNsis,
  [switch]$NoMsix
)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$root = Resolve-Path (Join-Path $here "..\..")
$assets = Join-Path $root "packaging\assets"
$Bin = (Resolve-Path $Bin).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must be a.b.c" }

foreach ($f in "uc-agent.exe", "uc.exe") {
  if (-not (Test-Path (Join-Path $Bin $f))) { throw "$f is missing in $Bin" }
}
$ui = Join-Path $Bin "Universal Clipboard.exe"
$hasUi = Test-Path $ui

# The files that go into both packages.
$stage = Join-Path $Out "stage-windows"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item (Join-Path $Bin "uc-agent.exe"), (Join-Path $Bin "uc.exe") $stage
if ($hasUi) { Copy-Item $ui $stage }
Copy-Item (Join-Path $root "LICENSE-MIT"), (Join-Path $root "LICENSE-APACHE") $stage

if (-not $NoNsis) {
  $makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
  if (-not $makensis) {
    foreach ($p in "${env:ProgramFiles(x86)}\NSIS\makensis.exe", "$env:ProgramFiles\NSIS\makensis.exe") {
      if (Test-Path $p) { $makensis = $p; break }
    }
  }
  if (-not $makensis) { throw "makensis not found (install NSIS 3)" }
  $setup = Join-Path $Out "UniversalClipboard-$Version-windows-$Arch-setup.exe"
  & $makensis /V2 "/DVERSION=$Version" "/DSRC=$stage" "/DOUT=$setup" (Join-Path $here "nsis\installer.nsi")
  if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
  Write-Host "NSIS: $setup"
}

if (-not $NoMsix) {
  $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
  $makeappx = Get-ChildItem $kits -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^10\.' } | Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName "x64\makeappx.exe" } | Where-Object { Test-Path $_ } |
    Select-Object -First 1
  if (-not $makeappx) { throw "makeappx not found (Windows SDK)" }
  $layout = Join-Path $Out "msix-layout"
  if (Test-Path $layout) { Remove-Item -Recurse -Force $layout }
  New-Item -ItemType Directory $layout, (Join-Path $layout "Assets") | Out-Null
  Copy-Item (Join-Path $stage "*") $layout
  Copy-Item (Join-Path $assets "icon-50.png") (Join-Path $layout "Assets\StoreLogo.png")
  Copy-Item (Join-Path $assets "icon-150.png") (Join-Path $layout "Assets\Square150x150Logo.png")
  Copy-Item (Join-Path $assets "icon-44.png") (Join-Path $layout "Assets\Square44x44Logo.png")
  $settingsApp = ""
  if ($hasUi) {
    $settingsApp = @'
    <Application Id="Settings" Executable="Universal Clipboard.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Universal Clipboard" Description="Copy on one device, paste on another" BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png" />
    </Application>
'@
  }
  $m = Get-Content -Raw (Join-Path $here "msix\AppxManifest.xml.in")
  $m = $m.Replace("@VERSION@", "$Version.0").Replace("@ARCH@", $Arch).Replace("@PUBLISHER@", $Publisher)
  $m = $m.Replace("@PUBLISHER_NAME@", $PublisherName).Replace("@SETTINGS_APP@", $settingsApp)
  Set-Content -Path (Join-Path $layout "AppxManifest.xml") -Value $m -Encoding utf8
  $msix = Join-Path $Out "UniversalClipboard-$Version-windows-$Arch.msix"
  & $makeappx pack /o /h SHA256 /d $layout /p $msix
  if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }
  Write-Host "MSIX: $msix"
}
