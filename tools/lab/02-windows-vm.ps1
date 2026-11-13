# Creates uc-win-b (Windows 11) with an unattended install. Needs a Windows 11 ISO (-IsoPath). Run elevated.
param([Parameter(Mandatory)][string]$IsoPath, [string]$UnattendTemplate = "$PSScriptRoot\autounattend.xml.template")
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabAdmin
$root = Get-LabRoot
$keys = (Get-LabPubKeys) -join "`n"
$stage = Join-Path $root 'uc-win-b-seed'
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $UnattendTemplate "$stage\autounattend.xml" -Force
# Post-install: OpenSSH server + authorized keys for admin user "uc". Auto-logon is set in the answer file.
$post = @"
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Set-Service sshd -StartupType Automatic; Start-Service sshd
New-NetFirewallRule -Name sshd -DisplayName sshd -Enabled True -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow -ErrorAction SilentlyContinue
`$k = @'
$keys
'@
Set-Content -Path C:\ProgramData\ssh\administrators_authorized_keys -Value `$k -Encoding ascii
icacls C:\ProgramData\ssh\administrators_authorized_keys /inheritance:r /grant 'SYSTEM:F' /grant 'Administrators:F' | Out-Null
powercfg /change standby-timeout-ac 0; powercfg /change monitor-timeout-ac 0
"@
Set-Content "$stage\uc-post.ps1" $post -Encoding ascii
$seed = Join-Path $root 'uc-win-b-seed.iso'
New-LabSeedIso -Folder $stage -OutIso $seed -Label 'UCSEED'
New-LabVm -Name 'uc-win-b' -InstallIso $IsoPath -SeedIso $seed -Cpu 4 -Windows
Write-Host 'uc-win-b started headless.'
