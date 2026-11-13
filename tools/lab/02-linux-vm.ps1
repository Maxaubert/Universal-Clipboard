# Creates an Ubuntu (GNOME) or Kubuntu (KDE) lab VM with a fully unattended install.
# Uses the Ubuntu Server ISO with autoinstall, then installs the desktop package set
# (more reliable to script than the desktop installer). Run elevated.
param(
    [Parameter(Mandatory)][ValidateSet('uc-lin-gnome','uc-lin-kde')][string]$Name,
    [string]$IsoPath,          # optional: an existing Ubuntu live-server ISO
    [string]$Release = '26.04'
)
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabAdmin
$root = Get-LabRoot
if (-not $IsoPath) {
    $base = "https://releases.ubuntu.com/$Release"
    $sums = (Invoke-WebRequest "$base/SHA256SUMS" -UseBasicParsing).Content
    $file = (($sums -split "`n" | Where-Object { $_ -match 'live-server-amd64\.iso' } | Select-Object -First 1) -replace '^\S+\s+\*?', '').Trim()
    $IsoPath = Join-Path $root $file
    if (-not (Test-Path $IsoPath)) { Write-Host "Downloading $file"; Start-BitsTransfer "$base/$file" $IsoPath }
}
$kde  = $Name -eq 'uc-lin-kde'
$pkgs = if ($kde) { 'kubuntu-desktop' } else { 'ubuntu-desktop-minimal' }
$dm   = if ($kde) { 'sddm' } else { 'gdm3' }
$keys = (Get-LabPubKeys | ForEach-Object { "      - `"$_`"" }) -join "`n"
# Fallback console password "uc-lab". SSH password login is off; keys only.
$openssl = 'C:\Program Files\Git\usr\bin\openssl.exe'
$hash = (& $openssl passwd -6 -salt uclabsalt 'uc-lab').Trim()
$ud = @"
#cloud-config
autoinstall:
  version: 1
  locale: en_US.UTF-8
  keyboard: { layout: us }
  identity: { hostname: $Name, username: uc, password: "$hash" }
  ssh:
    install-server: true
    allow-pw: false
    authorized-keys:
$keys
  storage: { layout: { name: lvm } }
  packages: [ $pkgs, openssh-server, hyperv-daemons, wl-clipboard, xclip, xdotool, wtype, ydotool, dbus-x11 ]
  late-commands:
    - echo 'uc ALL=(ALL) NOPASSWD:ALL' > /target/etc/sudoers.d/90-uc
    - chmod 440 /target/etc/sudoers.d/90-uc
    - curtin in-target --target=/target -- systemctl enable ssh
    - curtin in-target --target=/target -- systemctl set-default graphical.target
    - bash -c "$(if ($kde) { 'mkdir -p /target/etc/sddm.conf.d; printf ''[Autologin]\nUser=uc\nSession=plasma\n'' > /target/etc/sddm.conf.d/autologin.conf' } else { 'mkdir -p /target/etc/gdm3; printf ''[daemon]\nAutomaticLoginEnable=true\nAutomaticLogin=uc\nWaylandEnable=true\n'' > /target/etc/gdm3/custom.conf' })"
"@
$stage = Join-Path $root "$Name-seed"
New-Item -ItemType Directory -Force $stage | Out-Null
[IO.File]::WriteAllText("$stage\user-data", ($ud -replace "`r`n", "`n"))
[IO.File]::WriteAllText("$stage\meta-data", "instance-id: $Name`n")
$seed = Join-Path $root "$Name-seed.iso"
New-LabSeedIso -Folder $stage -OutIso $seed -Label 'CIDATA'
New-LabVm -Name $Name -InstallIso $IsoPath -SeedIso $seed -Cpu 4
Write-Host "$Name started headless. Install takes 15-30 min. Then run .\03-status.ps1"
