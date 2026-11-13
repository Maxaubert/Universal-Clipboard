# Prints state and IP of each lab VM.
$rows = foreach ($n in 'uc-win-b','uc-lin-gnome','uc-lin-kde') {
    $vm = Get-VM -Name $n -ErrorAction SilentlyContinue
    $ip = (Get-VMNetworkAdapter -VMName $n -ErrorAction SilentlyContinue).IPAddresses | Where-Object { $_ -match '^\d+\.\d+\.\d+\.\d+$' } | Select-Object -First 1
    [pscustomobject]@{ Name = $n; State = $vm.State; Ip = $ip }
}
$rows | Format-Table -AutoSize
