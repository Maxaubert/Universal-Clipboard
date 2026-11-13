# Shared helpers for the Universal Clipboard test lab (plan section 6.0).
$ErrorActionPreference = 'Stop'
$script:LabRoot = 'D:\uc-lab'
if (-not (Test-Path 'D:\')) { $script:LabRoot = 'C:\uc-lab' }

function Assert-LabAdmin {
    $p = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    if (-not $p.IsInRole('Administrator')) { throw 'Run this from an elevated PowerShell.' }
    if ((Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All).State -ne 'Enabled') {
        throw 'Hyper-V is not enabled. Enable it and reboot first.'
    }
}

function Get-LabRoot { New-Item -ItemType Directory -Force $script:LabRoot | Out-Null; $script:LabRoot }

function Get-LabPubKeys {
    # ONLY the lab key id_ed25519_hyperv.pub ("claude-hyperv-ubuntu"). Never id_ed25519 ("pc-to-mac") or any other key.
    $f = "$env:USERPROFILE\.ssh\id_ed25519_hyperv.pub"
    if (-not (Test-Path $f)) { throw "Lab key not found: $f" }
    ,@((Get-Content $f -Raw).Trim())
}

function New-LabSeedIso {
    # Builds an ISO9660/Joliet image from a folder using IMAPI2 (no ADK needed).
    param([Parameter(Mandatory)][string]$Folder, [Parameter(Mandatory)][string]$OutIso, [string]$Label = 'CIDATA')
    if (Test-Path $OutIso) { Remove-Item $OutIso -Force }
    $fs = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
    $fs.FileSystemsToCreate = 3
    $fs.VolumeName = $Label
    $fs.Root.AddTree($Folder, $false)
    $stream = $fs.CreateResultImage().ImageStream
    if (-not ('IsoWriter' -as [type])) {
        Add-Type -TypeDefinition @'
using System; using System.IO; using System.Runtime.InteropServices; using System.Runtime.InteropServices.ComTypes;
public static class IsoWriter { public static void Write(object comStream, string path) {
  IStream s = (IStream)comStream; STATSTG st; s.Stat(out st, 1);
  long total = st.cbSize; byte[] buf = new byte[1048576]; IntPtr pRead = Marshal.AllocHGlobal(4);
  using (var f = File.Create(path)) { long done = 0; while (done < total) { s.Read(buf, buf.Length, pRead); int n = Marshal.ReadInt32(pRead); if (n <= 0) break; f.Write(buf, 0, n); done += n; } }
  Marshal.FreeHGlobal(pRead); } }
'@
    }
    [IsoWriter]::Write($stream, $OutIso)
}

function New-LabVm {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$InstallIso,
        [string]$SeedIso,
        [int]$Cpu = 4,
        [switch]$Windows
    )
    if (Get-VM -Name $Name -ErrorAction SilentlyContinue) { Write-Host "$Name exists, skipping create"; return }
    $dir = Join-Path (Get-LabRoot) $Name
    New-Item -ItemType Directory -Force $dir | Out-Null
    $vhd = Join-Path $dir "$Name.vhdx"
    New-VHD -Path $vhd -SizeBytes 64GB -Dynamic | Out-Null
    $vm = New-VM -Name $Name -Generation 2 -MemoryStartupBytes 4GB -VHDPath $vhd -SwitchName 'uc-lab' -Path $dir
    Set-VM $vm -ProcessorCount $Cpu -DynamicMemory -MemoryMinimumBytes 2GB -MemoryMaximumBytes 4GB -AutomaticStartAction Nothing -AutomaticStopAction ShutDown -CheckpointType Disabled
    if ($Windows) {
        Set-VMFirmware $vm -EnableSecureBoot On
        Set-VMKeyProtector -VMName $Name -NewLocalKeyProtector
        Enable-VMTPM -VMName $Name
    } else {
        Set-VMFirmware $vm -EnableSecureBoot Off
    }
    Add-VMDvdDrive -VMName $Name -Path $InstallIso
    if ($SeedIso) { Add-VMDvdDrive -VMName $Name -Path $SeedIso }
    Set-VMFirmware $vm -FirstBootDevice (Get-VMDvdDrive -VMName $Name | Select-Object -First 1)
    Enable-VMIntegrationService -VMName $Name -Name 'Guest Service Interface'
    Start-VM $vm   # headless: no vmconnect window is ever opened
}
Export-ModuleMember -Function *
