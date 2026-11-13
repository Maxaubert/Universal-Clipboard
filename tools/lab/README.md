# Test lab (plan section 6.0)

Hyper-V VMs on the PC for cross-OS tests. All headless: the scripts never open vmconnect.

| VM | OS | Login | Notes |
|---|---|---|---|
| uc-win-b | Windows 11 | auto-logon user `uc` | ISO default `D:\uc-lab\iso\Win11_English_x64.iso` |
| uc-lin-gnome | Ubuntu (GNOME, Wayland) | auto-login `uc` | Server ISO autoinstall + `ubuntu-desktop-minimal` |
| uc-lin-kde | Kubuntu (KDE Plasma) | auto-login `uc` | Server ISO autoinstall + `kubuntu-desktop` |

Each VM: 4 GB RAM dynamic, 4 vCPU, 64 GB dynamic disk, external switch `uc-lab`, OpenSSH for user `uc`. The VMs trust ONLY `~/.ssh/id_ed25519_hyperv.pub` ("claude-hyperv-ubuntu"), never `id_ed25519` ("pc-to-mac"). On Windows the key goes in `administrators_authorized_keys`. VMs do NOT start with the host (AutomaticStartAction Nothing): tests start them on demand with `Start-VM`. No login items or scheduled tasks. Fallback console password: `uc-lab`.

Run from an **elevated** PowerShell:

```powershell
.\01-switch.ps1                    # once; network drops a few seconds
.\02-linux-vm.ps1 -Name uc-lin-gnome
.\02-linux-vm.ps1 -Name uc-lin-kde
.\02-windows-vm.ps1                # default ISO path above, or -IsoPath <file>
.\03-status.ps1                    # state and IPs
ssh uc@<ip>
```

Your account must be in the Hyper-V Administrators group (or run elevated).
VM files live in `D:\uc-lab` (or `C:\uc-lab` when there is no D: drive).
Status: scripts written but not yet run (the authoring session had no admin rights).
