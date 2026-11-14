# S7 spike runner. Runs every check against one headless emulator and saves logs and screenshots.
# Usage: .\run-spike.ps1 -Serial emulator-5556 -Out ..\..\docs\spikes\s7\api37
# Safety: the emulator runs with -no-window, so there is no Qt clipboard bridge to the host.
# The script still reads the Windows clipboard sequence number (read-only) before and after,
# to prove that the host clipboard was not changed.
param([Parameter(Mandatory)] [string]$Serial, [Parameter(Mandatory)] [string]$Out)
$ErrorActionPreference = 'Continue'
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"
$apk = Join-Path $PSScriptRoot 'app\build\outputs\apk\debug\app-debug.apk'
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Add-Type -Namespace W -Name U -MemberDefinition '[DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();'
$seq0 = [W.U]::GetClipboardSequenceNumber()

function A { & $adb -s $Serial @args }
function Shot([string]$name) {
    A shell screencap -p /sdcard/s7.png | Out-Null
    A pull /sdcard/s7.png (Join-Path $Out "$name.png") | Out-Null
}
function Bc([string]$action, [string[]]$extra = @()) {
    A shell am broadcast -a $action -p dev.uc.s7 @extra | Out-Null
}
function Note([string]$t) { Add-Content (Join-Path $Out 'steps.txt') "$(Get-Date -Format HH:mm:ss) $t"; Write-Host $t }

Remove-Item (Join-Path $Out 'steps.txt') -ErrorAction SilentlyContinue
$sdk = [int](A shell getprop ro.build.version.sdk).Trim()
Note "serial=$Serial sdk=$sdk release=$((A shell getprop ro.build.version.release).Trim())"
A install -r -g $apk | Out-Null
if ($sdk -ge 33) { A shell pm grant dev.uc.s7 android.permission.POST_NOTIFICATIONS }
A shell input keyevent KEYCODE_WAKEUP; A shell wm dismiss-keyguard; A shell input keyevent KEYCODE_HOME
A logcat -c
Start-Sleep 1

Note '1. open app (starts FGS), then go home'
A shell am start -W -n dev.uc.s7/.MainActivity | Out-Null
Start-Sleep 3
A shell input keyevent KEYCODE_HOME
Start-Sleep 2
$fgs = (A shell dumpsys activity services dev.uc.s7) -match 'isForeground|foregroundServiceType|ServiceRecord'
Note ("FGS state: " + (($fgs | % { $_.Trim() }) -join ' | '))
$imp = (A shell dumpsys activity processes dev.uc.s7) -match 'procState|curProcState|ProcessRecord' | Select -First 3
Note ("proc: " + (($imp | % { $_.Trim() }) -join ' | '))
Shot '01-home-baseline'

Note '2. background write, plain clip'
Bc dev.uc.s7.WRITE @('--es', 'text', "bg-plain-$sdk")
Start-Sleep -Milliseconds 700; Shot '02-after-plain-write'

Note '3. background write, with SHOW_CLIPBOARD_OVERLAY (makes the emulator act like a phone)'
Bc dev.uc.s7.WRITE @('--es', 'text', "bg-overlay-$sdk", '--es', 'flags', 'overlay')
Start-Sleep -Milliseconds 700; Shot '03-overlay-write'
Start-Sleep 4; Shot '03b-overlay-write-5s'

Note '4. background write, overlay + EXTRA_IS_REMOTE_DEVICE'
Bc dev.uc.s7.WRITE @('--es', 'text', "bg-remote-$sdk", '--es', 'flags', 'overlay,remote')
Start-Sleep -Milliseconds 700; Shot '04-overlay-remote-write'
Start-Sleep 4

Note '5. background write, overlay + IS_SENSITIVE'
Bc dev.uc.s7.WRITE @('--es', 'text', "bg-sensitive-$sdk", '--es', 'flags', 'overlay,sensitive')
Start-Sleep -Milliseconds 700; Shot '05-overlay-sensitive-write'
Start-Sleep 4

Note '6. screen off, write after 3 s, screen on'
A shell input keyevent KEYCODE_SLEEP
Bc dev.uc.s7.WRITE @('--es', 'text', "bg-screenoff-$sdk", '--ei', 'delayMs', '3000')
Start-Sleep 5
A shell input keyevent KEYCODE_WAKEUP; A shell wm dismiss-keyguard
Start-Sleep 1

Note '7. forced Doze (deviceidle force-idle), write, unforce'
A shell dumpsys battery unplug | Out-Null
Note ("doze: " + ((A shell dumpsys deviceidle force-idle) -join ' '))
Bc dev.uc.s7.WRITE @('--es', 'text', "bg-doze-$sdk")
Start-Sleep 2
A shell dumpsys deviceidle unforce | Out-Null
A shell dumpsys battery reset | Out-Null
A shell input keyevent KEYCODE_WAKEUP; A shell wm dismiss-keyguard; A shell input keyevent KEYCODE_HOME
Start-Sleep 1

Note '8. background read from the service (expected: blocked)'
Bc dev.uc.s7.READ
Start-Sleep 1

Note '9. Rust library from the service'
Bc dev.uc.s7.NATIVE
Start-Sleep 1

Note '10. Quick Settings tile opens the transparent focus activity, which reads the clipboard'
$add = (A shell cmd statusbar add-tile dev.uc.s7/.ClipTile) -join ' '
Note "add-tile: $add"
Start-Sleep 2
$click = (A shell cmd statusbar click-tile dev.uc.s7/.ClipTile) -join ' '
Note "click-tile: $click"
Start-Sleep 3
Shot '10-after-tile'
A shell cmd statusbar collapse | Out-Null

Note '11. fallback: post the "Tap to copy" notification and open the shade'
Bc dev.uc.s7.TAP @('--es', 'text', "tap-$sdk")
Start-Sleep 1
A shell cmd statusbar expand-notifications | Out-Null
Start-Sleep 2
Shot '11-tap-to-copy-notification'
A shell cmd statusbar collapse | Out-Null

A logcat -d -v time -s S7:* ClipboardService:* ActivityManager:W AndroidRuntime:E *:F > (Join-Path $Out 'logcat.txt')
A logcat -d -v time > (Join-Path $Out 'logcat-full.txt')
$seq1 = [W.U]::GetClipboardSequenceNumber()
Note "host clipboard sequence before=$seq0 after=$seq1 (equal means the Windows clipboard was not touched)"
