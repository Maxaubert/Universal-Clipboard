# Test lab (plan section 6.0)

Two Linux Hyper-V VMs on the PC for the Linux release gate. Windows and Mac tests use Max's real PC
and Mac (decision 2026-11-14: Linux VMs only; `win-b` and `mac-b` are dropped).
Everything is headless: the scripts never open vmconnect or Hyper-V Manager.

| VM | Desktop | Session |
|---|---|---|
| `lin-gnome` | Ubuntu 26.04.1, GNOME | Wayland, auto-login `lab` (GDM) |
| `lin-kde` | Ubuntu 26.04.1 + `kubuntu-desktop`, Plasma 6 | Wayland, auto-login `lab` (SDDM) |

Each VM: Gen2, Secure Boot (template `MicrosoftUEFICertificateAuthority`), 4 vCPU, 4-8 GB dynamic RAM,
60 GB dynamic VHDX, external switch `uc-lan` (bridged to the PC's Ethernet, so the VMs are real LAN
peers with their own DHCP address and mDNS). HWE kernel plus `linux-cloud-tools` (hv_kvp), so
`Get-VMNetworkAdapter` shows the IP. Installed: OpenSSH, `wl-clipboard`, `xclip`, avahi (`avahi-utils`,
`libnss-mdns`).

**Why not the Kubuntu ISO:** it uses the Calamares installer, which has no unattended mode. Both VMs
install from the Ubuntu desktop ISO (subiquity autoinstall); `lin-kde` then installs `kubuntu-desktop`
and SDDM and removes GDM and GNOME Shell. The result is the Kubuntu package set on Plasma 6 Wayland.

## Access and safety

- User `lab`, passwordless sudo. SSH accepts ONLY `~/.ssh/id_ed25519_hyperv` ("claude-hyperv-ubuntu");
  password and keyboard-interactive login over SSH are off. Console fallback password: `uc-lab`
  (Hyper-V console only).
- The VMs never start with the PC (AutomaticStartAction Nothing, AutomaticStopAction ShutDown). Tests
  start them with `Start-VM` and shut them down after.
- No automatic upgrades or popups: `unattended-upgrades` and `update-notifier` are removed, the apt
  timers are masked, snap refreshes are held, sleep and the screen lock are off.
- Host keys go to `X:\uc-lab\known_hosts`, never `~/.ssh/known_hosts`.

## Build

Needs: Hyper-V Administrators membership (no elevation), `xorriso` in WSL Ubuntu, and the checksum-
verified ISO `X:\uc-lab\iso\ubuntu-26.04.1-desktop-amd64.iso`. Lab root: `X:\uc-lab` (or `$env:UC_LAB_ROOT`).

```powershell
.\01-switch.ps1      # once; creates uc-lan, the PC network drops a few seconds
.\02-vm.ps1          # both VMs in parallel (or -Name lin-kde); install ~15 min, ends powered off
.\04-finalize.ps1    # detach media, boot, wait for IP + SSH, verify, checkpoint 'clean', shut down
.\03-status.ps1      # state, IP, MAC, checkpoints
.\05-remove.ps1      # delete the lab VMs (only lin-*) to rebuild; -Iso also drops the remastered ISO
```

How it is unattended: `02-vm.ps1` copies the Ubuntu ISO once (`*-autoinstall.iso`) with a grub.cfg that
puts `autoinstall ds=nocloud noprompt noeject` on the kernel line, and gives each VM a NoCloud seed ISO
(label `CIDATA`, `user-data` with `autoinstall:`). `noprompt noeject` stops casper from hanging on
"remove the install medium" at the final power-off.

To look at a console without a window: `Import-Module .\Lab.psm1; Save-LabScreenshot lin-kde C:\tmp\kde.png`.

Back to clean before a test run:

```powershell
Restore-VMSnapshot -VMName lin-gnome -Name clean -Confirm:$false; Start-VM lin-gnome
```

## SSH config (optional)

The scripts do not edit `~/.ssh/config`. To use short names, add this yourself (IPs come from DHCP;
check them with `.\03-status.ps1`, or use the mDNS names):

```
Host lin-gnome lin-kde
    HostName %h.local
    User lab
    IdentityFile ~/.ssh/id_ed25519_hyperv
    IdentitiesOnly yes
    UserKnownHostsFile X:/uc-lab/known_hosts
```

The PC resolves `.local` names only if Windows mDNS answers for them; otherwise use the IP.
Status: built and verified 2026-11-14.
