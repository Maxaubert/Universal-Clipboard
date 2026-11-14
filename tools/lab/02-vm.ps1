# Creates lin-gnome (Ubuntu, GNOME on Wayland) and/or lin-kde (Plasma 6 on Wayland) with a fully
# unattended install. Both use the Ubuntu 26.04 desktop ISO (subiquity autoinstall; the Kubuntu ISO uses
# Calamares, which has no unattended mode), so lin-kde installs kubuntu-desktop and SDDM on top.
# Autoinstall config comes from a NoCloud seed ISO (label CIDATA). The install ISO is a copy with
# 'autoinstall' on the kernel line, so nothing asks for confirmation. The VMs power off when done;
# then run .\04-finalize.ps1. No elevation needed (Hyper-V Administrators is enough).
param([ValidateSet('lin-gnome', 'lin-kde')][string[]]$Name = @('lin-gnome', 'lin-kde'))
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabHyperV
$root = Get-LabRoot
$iso  = New-LabInstallIso
$key  = Get-LabPubKey
# Console fallback password "uc-lab" (the Hyper-V console only). SSH accepts the lab key only.
$hash = (& 'C:\Program Files\Git\usr\bin\openssl.exe' passwd -6 'uc-lab').Trim()

function B64([string]$s) { [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($s -replace "`r`n", "`n"))) }
function WriteFile([string]$path, [string]$body, [string]$mode = '644') {
    "    - mkdir -p /target$(Split-Path $path -Parent | ForEach-Object { $_ -replace '\\', '/' }) && echo $(B64 $body) | base64 -d > /target$path && chmod $mode /target$path"
}
$inTarget = 'curtin in-target --target=/target -- env DEBIAN_FRONTEND=noninteractive'

foreach ($n in $Name) {
    $kde = $n -eq 'lin-kde'
    $lc = [Collections.Generic.List[string]]::new()
    $lc.Add((WriteFile '/etc/sudoers.d/90-lab' "lab ALL=(ALL) NOPASSWD:ALL`n" '440'))
    $lc.Add((WriteFile '/etc/ssh/sshd_config.d/10-lab.conf' "PasswordAuthentication no`nKbdInteractiveAuthentication no`nPermitRootLogin no`n"))
    $lc.Add((WriteFile '/etc/apt/apt.conf.d/20auto-upgrades' "APT::Periodic::Update-Package-Lists `"0`";`nAPT::Periodic::Unattended-Upgrade `"0`";`n"))
    $lc.Add("    - $inTarget apt-get update")
    # Hyper-V integration: hv_kvp_daemon (reports the IP to Get-VMNetworkAdapter) must match the kernel, so
    # install the HWE kernel and its cloud tools together.
    $lc.Add("    - $inTarget apt-get install -y linux-generic-hwe-26.04 linux-cloud-tools-generic-hwe-26.04 openssh-server wl-clipboard xclip avahi-daemon avahi-utils libnss-mdns")
    $lc.Add("    - $inTarget apt-get purge -y unattended-upgrades update-notifier gnome-initial-setup || true")
    if ($kde) {
        $lc.Add("    - $inTarget sh -c 'echo sddm shared/default-x-display-manager select sddm | debconf-set-selections'")
        $lc.Add("    - $inTarget apt-get install -y kubuntu-desktop sddm")
        $lc.Add("    - $inTarget apt-get purge -y gdm3 gnome-shell ubuntu-session || true")
        $lc.Add((WriteFile '/etc/X11/default-display-manager' "/usr/bin/sddm`n"))
        # 99-: Kubuntu's 20-kubuntu.conf sets an empty Autologin User, and later files win.
        $lc.Add((WriteFile '/etc/sddm.conf.d/99-lab-autologin.conf' "[Autologin]`nUser=lab`nSession=plasma`nRelogin=true`n"))
        $lc.Add((WriteFile '/etc/xdg/kscreenlockerrc' "[Daemon]`nAutolock=false`nLockOnResume=false`n"))
        $lc.Add("    - curtin in-target --target=/target -- systemctl enable sddm")
    } else {
        $lc.Add((WriteFile '/etc/gdm3/custom.conf' "[daemon]`nAutomaticLoginEnable=true`nAutomaticLogin=lab`nWaylandEnable=true`n"))
        $lc.Add((WriteFile '/etc/dconf/profile/user' "user-db:user`nsystem-db:local`n"))
        $lc.Add((WriteFile '/etc/dconf/db/local.d/00-lab' "[org/gnome/desktop/screensaver]`nlock-enabled=false`n`n[org/gnome/desktop/session]`nidle-delay=uint32 0`n`n[org/gnome/settings-daemon/plugins/power]`nsleep-inactive-ac-type='nothing'`n"))
        $lc.Add("    - curtin in-target --target=/target -- dconf update")
    }
    $lc.Add("    - curtin in-target --target=/target -- systemctl mask apt-daily.timer apt-daily-upgrade.timer sleep.target suspend.target hibernate.target hybrid-sleep.target")
    $lc.Add("    - curtin in-target --target=/target -- systemctl enable ssh avahi-daemon")
    $lc.Add("    - curtin in-target --target=/target -- systemctl set-default graphical.target")

    $ud = @"
#cloud-config
autoinstall:
  version: 1
  interactive-sections: []
  refresh-installer: { update: false }
  source: { id: ubuntu-desktop-minimal }
  locale: en_US.UTF-8
  keyboard: { layout: us }
  timezone: Europe/Oslo
  identity: { hostname: $n, realname: Lab, username: lab, password: "$hash" }
  ssh:
    install-server: true
    allow-pw: false
    authorized-keys:
      - "$key"
  storage: { layout: { name: direct } }
  codecs: { install: false }
  drivers: { install: false }
  updates: security
  shutdown: poweroff
  late-commands:
$($lc -join "`n")
"@
    $stage = Join-Path $root "seed\$n"
    New-Item -ItemType Directory -Force $stage | Out-Null
    [IO.File]::WriteAllText("$stage\user-data", ($ud -replace "`r`n", "`n"))
    [IO.File]::WriteAllText("$stage\meta-data", "instance-id: $n`nlocal-hostname: $n`n")
    $seed = Join-Path $root "seed\$n-seed.iso"
    New-LabSeedIso -Folder $stage -OutIso $seed -Label 'CIDATA'
    New-LabVm -Name $n -InstallIso $iso -SeedIso $seed
    Write-Host "$n started headless. The install takes 20-40 min and ends with the VM off. Then run .\04-finalize.ps1"
}
