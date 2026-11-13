# Test lab (plan section 6.0)

Hyper-V VMs on the PC for cross-OS tests. All headless: the scripts never open vmconnect.

| VM | OS | Login | Notes |
|---|---|---|---|
| uc-win-b | Windows 11 | auto-logon user `uc` | needs a Windows 11 ISO |
| uc-lin-gnome | Ubuntu (GNOME, Wayland) | auto-login `uc` | Server ISO autoinstall + `ubuntu-desktop-minimal` |
| uc-lin-kde | Kubuntu (KDE Plasma) | auto-login `uc` | Server ISO autoinstall + `kubuntu-desktop` |

Each VM: 4 GB RAM dynamic, 4 vCPU, 64 GB dynamic disk, external switch `uc-lab`, OpenSSH with the PC's `~/.ssh/id_*.pub` keys for user `uc`. Fallback console password: `uc-lab`.

Run from an **elevated** PowerShell:

```powershell
.\01-switch.ps1                    # once; network drops a few seconds
.\02-linux-vm.ps1 -Name uc-lin-gnome
.\02-linux-vm.ps1 -Name uc-lin-kde
.\02-windows-vm.ps1 -IsoPath C:\path\Win11.iso
.\03-status.ps1                    # state and IPs
ssh uc@<ip>
```

VM files live in `D:\uc-lab` (or `C:\uc-lab` when there is no D: drive).
Status: scripts written but not yet run (the authoring session had no admin rights).
