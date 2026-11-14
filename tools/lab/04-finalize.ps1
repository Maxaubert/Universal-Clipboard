# Runs after 02-vm.ps1, once the install has powered the VM off. For each VM: removes the install media,
# boots from disk, waits for the IP (Hyper-V KVP) and SSH, verifies the session, takes the checkpoint
# 'clean' and shuts the VM down. Headless. Host keys go to <lab root>\known_hosts, never ~/.ssh.
param([ValidateSet('lin-gnome', 'lin-kde')][string[]]$Name = @('lin-gnome', 'lin-kde'), [int]$TimeoutMin = 20)
Import-Module "$PSScriptRoot\Lab.psm1" -Force
Assert-LabHyperV
$root = Get-LabRoot
$key  = "$env:USERPROFILE\.ssh\id_ed25519_hyperv"
$kh   = Join-Path $root 'known_hosts'
$sshArgs = @('-i', $key, '-o', 'IdentitiesOnly=yes', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5',
             '-o', 'StrictHostKeyChecking=accept-new', '-o', "UserKnownHostsFile=$kh")
$verify = @'
set +e
echo "host: $(hostname)  kernel: $(uname -r)"
echo "--- loginctl"; loginctl list-sessions --no-legend
for s in $(loginctl list-sessions --no-legend | awk '{print $1}'); do loginctl show-session "$s" -p Name -p Type -p Class -p State -p Desktop | tr '\n' ' '; echo; done
echo "--- tools"; command -v wl-copy wl-paste xclip avahi-resolve
echo "--- wayland clipboard"; export XDG_RUNTIME_DIR=/run/user/$(id -u); export WAYLAND_DISPLAY=$(basename "$(ls $XDG_RUNTIME_DIR/wayland-? 2>/dev/null | head -1)")
echo "WAYLAND_DISPLAY=$WAYLAND_DISPLAY"; echo uc-lab-check | wl-copy >/dev/null 2>&1; timeout 5 wl-paste   # wl-copy forks a server; keep it off the SSH pipe
echo "--- mdns"; avahi-resolve -4 -n "$(hostname).local"
echo "--- sshd"; sudo sshd -T | grep -E '^(passwordauthentication|kbdinteractiveauthentication|permitrootlogin|pubkeyauthentication) '
echo "authorized_keys: $(wc -l < ~/.ssh/authorized_keys) key(s): $(awk '{print $3}' ~/.ssh/authorized_keys)"
echo "--- services"; systemctl is-active hv-kvp-daemon ssh avahi-daemon display-manager
echo "--- upgrades"; systemctl is-enabled apt-daily.timer apt-daily-upgrade.timer; dpkg -l unattended-upgrades 2>/dev/null | grep -c ^ii
sudo snap refresh --hold >/dev/null 2>&1 && echo "snap refresh held"
for s in $(loginctl list-sessions --no-legend | awk '{print $1}'); do
  [ "$(loginctl show-session "$s" -p Name -p Type -p Class --value | tr '\n' ' ')" = "lab wayland user " ] && { echo "OK: lab has a Wayland session"; exit 0; }
done
echo "FAIL: no graphical Wayland session for lab"; exit 3
'@

function Wait-LabSsh([string]$n) {
    $deadline = (Get-Date).AddMinutes($TimeoutMin)
    while ((Get-Date) -lt $deadline) {
        $ip = Get-LabVmIp $n
        if ($ip) { ssh @sshArgs "lab@$ip" true 2>$null; if ($LASTEXITCODE -eq 0) { return $ip } }
        Start-Sleep 10
    }
    throw "$n did not answer on SSH within $TimeoutMin min."
}

foreach ($n in $Name) {
    $vm = Get-VM -Name $n
    if (Get-VMSnapshot -VMName $n -Name clean -ErrorAction SilentlyContinue) { Write-Host "$n already has 'clean'"; continue }
    $dvd = Get-VMDvdDrive -VMName $n
    if (-not $dvd) { if ($vm.State -eq 'Off') { Start-VM -Name $n }; continue }   # already booted from disk
    if ($vm.State -ne 'Off') { throw "$n is $($vm.State). Wait for the install to power it off." }
    $dvd | Remove-VMDvdDrive
    Set-VMFirmware -VMName $n -FirstBootDevice (Get-VMHardDiskDrive -VMName $n)
    Start-VM -Name $n
}
foreach ($n in $Name) {
    if (Get-VMSnapshot -VMName $n -Name clean -ErrorAction SilentlyContinue) { continue }
    $ip = Wait-LabSsh $n
    Start-Sleep 30   # let the auto-login session settle
    Write-Host "===== $n $ip $((Get-VMNetworkAdapter -VMName $n).MacAddress)"
    ($verify -replace "`r`n", "`n") | ssh @sshArgs "lab@$ip" 'bash -s'
    if ($LASTEXITCODE -ne 0) { throw "$n failed verification (exit $LASTEXITCODE); no checkpoint taken." }
    Stop-VM -Name $n             # guest shutdown, not power off
    Checkpoint-VM -Name $n -SnapshotName clean
}
& "$PSScriptRoot\03-status.ps1"
