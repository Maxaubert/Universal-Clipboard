# Shared helpers for the Universal Clipboard test lab (plan section 6.0).
# Two Linux VMs only: lin-gnome (Ubuntu, GNOME Wayland) and lin-kde (Plasma 6 Wayland).
$ErrorActionPreference = 'Stop'
$script:LabRoot   = if ($env:UC_LAB_ROOT) { $env:UC_LAB_ROOT } else { 'X:\uc-lab' }
$script:Switch    = 'uc-lan'
$script:UbuntuIso = 'ubuntu-26.04.1-desktop-amd64.iso'
$script:LabNames  = @('lin-gnome', 'lin-kde')

function Get-LabNames { $script:LabNames }
function Get-LabSwitchName { $script:Switch }

function Assert-LabHyperV {
    # Hyper-V Administrators membership is enough; no elevation needed.
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p  = [Security.Principal.WindowsPrincipal]$id
    $hvAdmin = $id.Groups.Value -contains 'S-1-5-32-578'
    if (-not ($hvAdmin -or $p.IsInRole('Administrator'))) {
        throw 'Add your account to "Hyper-V Administrators" (or run elevated).'
    }
    if (-not (Get-Command New-VM -ErrorAction SilentlyContinue)) { throw 'Hyper-V PowerShell module not found.' }
}

function Get-LabRoot {
    foreach ($d in 'iso', 'vms', 'seed') { New-Item -ItemType Directory -Force (Join-Path $script:LabRoot $d) | Out-Null }
    $script:LabRoot
}

function Get-LabPubKey {
    # ONLY the lab key id_ed25519_hyperv.pub ("claude-hyperv-ubuntu"). Never id_ed25519 ("pc-to-mac") or any other key.
    $f = "$env:USERPROFILE\.ssh\id_ed25519_hyperv.pub"
    if (-not (Test-Path $f)) { throw "Lab key not found: $f" }
    (Get-Content $f -Raw).Trim()
}

function ConvertTo-WslPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    '/mnt/' + $full.Substring(0, 1).ToLower() + ($full.Substring(2) -replace '\\', '/')
}

function New-LabInstallIso {
    # Copies the Ubuntu desktop ISO with a new grub.cfg: 'autoinstall ds=nocloud noprompt noeject' on the kernel line and a
    # 1 s menu, so subiquity runs without asking. Needs xorriso in WSL Ubuntu. Boot files are replayed as-is,
    # so the signed shim keeps Secure Boot working.
    $root = Get-LabRoot
    $src  = Join-Path $root "iso\$script:UbuntuIso"
    $dst  = Join-Path $root ('iso\' + ($script:UbuntuIso -replace '\.iso$', '-autoinstall.iso'))
    if (Test-Path $dst) { return $dst }
    if (-not (Test-Path $src)) { throw "Ubuntu desktop ISO not found: $src" }
    $grub = @'
set timeout=1
set default=0
loadfont unicode
menuentry "Autoinstall Ubuntu (uc-lab)" {
    set gfxpayload=keep
    linux  /casper/vmlinuz autoinstall ds=nocloud noprompt noeject --- quiet
    initrd /casper/initrd
}
'@
    $g = Join-Path $root 'seed\grub.cfg'
    [IO.File]::WriteAllText($g, ($grub -replace "`r`n", "`n"))
    $cmd = "xorriso -report_about WARNING -indev '$(ConvertTo-WslPath $src)' -outdev '$(ConvertTo-WslPath $dst)' " +
           "-map '$(ConvertTo-WslPath $g)' /boot/grub/grub.cfg -boot_image any replay"
    wsl.exe -d Ubuntu -- bash -lc $cmd
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $dst)) { throw 'xorriso remaster failed (is xorriso installed in WSL Ubuntu?).' }
    $dst
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
    # Gen2, Secure Boot (Microsoft UEFI CA), 4 vCPU, 4-8 GB dynamic RAM, 60 GB dynamic disk, on uc-lan.
    # Never starts with the host. Headless: no vmconnect window is ever opened.
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$InstallIso,
        [Parameter(Mandatory)][string]$SeedIso
    )
    if (Get-VM -Name $Name -ErrorAction SilentlyContinue) { throw "$Name exists. Remove it first to rebuild." }
    if (-not (Get-VMSwitch -Name $script:Switch -ErrorAction SilentlyContinue)) { throw "Switch $script:Switch missing. Run 01-switch.ps1." }
    $dir = Join-Path (Get-LabRoot) 'vms'
    $vhd = Join-Path $dir "$Name\$Name.vhdx"
    New-Item -ItemType Directory -Force (Split-Path $vhd) | Out-Null
    New-VHD -Path $vhd -SizeBytes 60GB -Dynamic | Out-Null
    $vm = New-VM -Name $Name -Generation 2 -MemoryStartupBytes 4GB -VHDPath $vhd -SwitchName $script:Switch -Path $dir
    Set-VM $vm -ProcessorCount 4 -DynamicMemory -MemoryMinimumBytes 4GB -MemoryMaximumBytes 8GB `
        -AutomaticStartAction Nothing -AutomaticStopAction ShutDown -CheckpointType Standard
    Set-VMFirmware $vm -EnableSecureBoot On -SecureBootTemplate MicrosoftUEFICertificateAuthority
    Add-VMDvdDrive -VMName $Name -Path $InstallIso
    Add-VMDvdDrive -VMName $Name -Path $SeedIso
    Set-VMFirmware $vm -FirstBootDevice (Get-VMDvdDrive -VMName $Name | Select-Object -First 1)
    Enable-VMIntegrationService -VMName $Name -Name 'Guest Service Interface'
    Start-VM $vm
}

function Get-LabVmIp([string]$Name) {
    (Get-VMNetworkAdapter -VMName $Name -ErrorAction SilentlyContinue).IPAddresses |
        Where-Object { $_ -match '^\d+\.\d+\.\d+\.\d+$' -and $_ -notlike '169.254.*' } | Select-Object -First 1
}
function Save-LabScreenshot {
    # Headless look at a VM console (WMI thumbnail, RGB565). Never opens vmconnect.
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$OutPng, [int]$W = 1024, [int]$H = 768)
    Add-Type -AssemblyName System.Drawing
    $ns  = 'root\virtualization\v2'
    $cs  = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$Name'"
    $sd  = Get-CimAssociatedInstance -InputObject $cs -ResultClassName Msvm_VirtualSystemSettingData |
        Where-Object VirtualSystemType -eq 'Microsoft:Hyper-V:System:Realized'
    $svc = Get-CimInstance -Namespace $ns -ClassName Msvm_VirtualSystemManagementService
    $r   = Invoke-CimMethod -InputObject $svc -MethodName GetVirtualSystemThumbnailImage -Arguments @{
        TargetSystem = $sd; WidthPixels = [uint16]$W; HeightPixels = [uint16]$H }
    $b   = [byte[]]$r.ImageData
    $bmp = New-Object Drawing.Bitmap($W, $H, [Drawing.Imaging.PixelFormat]::Format16bppRgb565)
    $d   = $bmp.LockBits((New-Object Drawing.Rectangle(0, 0, $W, $H)), 'WriteOnly', $bmp.PixelFormat)
    for ($y = 0; $y -lt $H -and ($y + 1) * $W * 2 -le $b.Length; $y++) {
        [Runtime.InteropServices.Marshal]::Copy($b, $y * $W * 2, [IntPtr]($d.Scan0.ToInt64() + $y * $d.Stride), $W * 2)
    }
    $bmp.UnlockBits($d)
    $bmp.Save($OutPng, [Drawing.Imaging.ImageFormat]::Png)
}
Export-ModuleMember -Function *
