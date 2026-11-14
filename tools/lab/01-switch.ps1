# Creates the external switch "uc-lan" on the physical adapter that carries the default route.
# The host network drops for a few seconds, once. Hyper-V Administrators is enough (no elevation).
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabHyperV
$sw = Get-LabSwitchName
if (Get-VMSwitch -Name $sw -ErrorAction SilentlyContinue) { Write-Host "$sw exists"; return }
$route = Get-NetRoute -DestinationPrefix 0.0.0.0/0 | Sort-Object RouteMetric | Select-Object -First 1
$nic = Get-NetAdapter -Physical | Where-Object { $_.Status -eq 'Up' -and $_.ifIndex -eq $route.ifIndex }
if (-not $nic) { throw 'The default route is not on an Up physical adapter (already bridged, or on Wi-Fi or VPN).' }
Write-Host "Binding $sw to $($nic.Name) ($($nic.InterfaceDescription))"
New-VMSwitch -Name $sw -NetAdapterName $nic.Name -AllowManagementOS $true | Out-Null
Start-Sleep 10
Get-NetIPAddress -AddressFamily IPv4 -InterfaceAlias "vEthernet ($sw)" | Format-Table InterfaceAlias, IPAddress, PrefixOrigin
Test-NetConnection github.com -Port 443 | Select-Object ComputerName, TcpTestSucceeded
