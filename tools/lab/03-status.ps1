# Prints state, IP, MAC and checkpoints of each lab VM.
Import-Module "$PSScriptRoot\Lab.psm1" -Force
$rows = foreach ($n in Get-LabNames) {
    $vm = Get-VM -Name $n -ErrorAction SilentlyContinue
    $na = Get-VMNetworkAdapter -VMName $n -ErrorAction SilentlyContinue
    [pscustomobject]@{
        Name        = $n
        State       = if ($vm) { $vm.State } else { 'missing' }
        Ip          = Get-LabVmIp $n
        Mac         = $na.MacAddress
        Checkpoints = if ($vm) { (Get-VMSnapshot -VMName $n | ForEach-Object Name) -join ',' } else { '' }
    }
}
$rows | Format-Table -AutoSize
