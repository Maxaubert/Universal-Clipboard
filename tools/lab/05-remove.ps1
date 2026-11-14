# Deletes lab VMs (lin-gnome / lin-kde only) and their disks, so 02-vm.ps1 can rebuild them.
# Never touches any other VM. Keeps the ISOs. -Iso also drops the remastered install ISO.
param([ValidateSet('lin-gnome', 'lin-kde')][string[]]$Name = @('lin-gnome', 'lin-kde'), [switch]$Iso)
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabHyperV
$root = Get-LabRoot
foreach ($n in $Name) {
    $vm = Get-VM -Name $n -ErrorAction SilentlyContinue
    if ($vm) {
        if ($vm.State -ne 'Off') { Stop-VM -VM $vm -TurnOff -Force }
        Get-VMSnapshot -VM $vm | Remove-VMSnapshot
        Remove-VM -VM $vm -Force
    }
    foreach ($p in (Join-Path $root "vms\$n"), (Join-Path $root "seed\$n"), (Join-Path $root "seed\$n-seed.iso")) {
        if (Test-Path $p) { Remove-Item $p -Recurse -Force }
    }
    Write-Host "$n removed"
}
if ($Iso) { Remove-Item (Join-Path $root 'iso\*-autoinstall.iso') -Force -ErrorAction SilentlyContinue }
