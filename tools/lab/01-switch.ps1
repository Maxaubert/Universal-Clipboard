# Creates the external switch "uc-lab" on the active physical adapter.
# The network drops for a few seconds, once. Run elevated.
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabAdmin
if (Get-VMSwitch -Name uc-lab -ErrorAction SilentlyContinue) { Write-Host 'uc-lab exists'; return }
$nic = Get-NetAdapter -Physical | Where-Object { $_.Status -eq 'Up' -and $_.InterfaceDescription -notmatch 'Virtual|Hyper-V|Mullvad|Wintun|TAP' } |
    Sort-Object ReceiveLinkSpeed -Descending | Select-Object -First 1
if (-not $nic) { throw 'No active physical adapter found.' }
Write-Host "Binding uc-lab to $($nic.Name)"
New-VMSwitch -Name uc-lab -NetAdapterName $nic.Name -AllowManagementOS $true | Out-Null
