; Universal Clipboard: per-user Windows installer (plan 2.1, 8.1, 8.5). It installs without an
; administrator; only the firewall rule asks Windows for one UAC prompt (see below).
;
; Build: makensis /DVERSION=0.1.0 /DSRC=<folder with the exes> /DOUT=<setup.exe> installer.nsi
; (packaging/windows/build.ps1 does this.)
;
; Switches:
;   /S                 silent
;   /UPDATE            used by the updater: do not stop the agent (the helper did), do not
;                      start anything at the end
;   /ADDFIREWALL       also add the firewall rule in a silent install (/S). Without it a
;                      silent install never shows a UAC prompt. The updater does not pass it:
;                      the rule is tied to the uc-agent.exe path, which an update keeps.
;   /REMOVEFIREWALL    uninstaller only: also remove the firewall rule without asking
;   /D=<folder>        install folder (last, no quotes)
;
; The logic lives in uc-agent (tested in Rust): --install-hook puts "uc" on PATH (or "uclip"
; when a "uc" already exists), --uninstall-hook tells the group, stops the agent and removes
; start at login, the state folder and the keys.
;
; Firewall rule (decision 2026-11-14, Max): the app must not show a system prompt after the
; install. So the installer adds the inbound rule (Private, LocalSubnet, UDP, for uc-agent.exe)
; with "uc-agent --firewall add", which elevates only itself: one UAC prompt during an
; interactive install, none when the rule is there already. "No" (exit 3) or a failure (exit 1)
; does not stop the install: the agent then runs outbound-only and shows a note with the fix
; ("uc firewall add"). The install itself stays per user. The uninstaller offers removal.

Unicode true
SetCompressor /SOLID lzma

!ifndef VERSION
  !error "VERSION is not set"
!endif
!ifndef SRC
  !error "SRC is not set"
!endif
!ifndef OUT
  !define OUT "UniversalClipboard-${VERSION}-setup.exe"
!endif

!define NAME "Universal Clipboard"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\UniversalClipboard"
!define UI_EXE "Universal Clipboard.exe"
!define PRIVACY "https://maxaubert.github.io/Universal-Clipboard/privacy"

Name "${NAME}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Universal Clipboard"
RequestExecutionLevel user
ShowInstDetails show
ShowUninstDetails show
BrandingText "${NAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "FileDescription" "${NAME} setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!define MUI_ICON "..\..\assets\icon.ico"
!define MUI_UNICON "..\..\assets\icon.ico"
!define MUI_ABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "${NAME}"
!define MUI_WELCOMEPAGE_TEXT "Copy on one device, paste on another.$\r$\n$\r$\nThis installs ${NAME} for you only. Windows asks for permission once, to let your other devices connect to this PC through the firewall.$\r$\n$\r$\nNo account, no server. Your clipboard only moves between your own devices on your local network.$\r$\n$\r$\nPrivacy: ${PRIVACY}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open ${NAME}"
!define MUI_FINISHPAGE_RUN_FUNCTION OpenApp
!define MUI_FINISHPAGE_TEXT "${NAME} is installed.$\r$\n$\r$\nWithout the window, open a new terminal and run: uc pair show"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Var IsUpdate
Var AddFirewall
Var Busy
Var Tries
Var PS

; Closing the app before its files are replaced (bug 2026-11-14). The old order (stop the
; agent, then close the window) let the open window start the agent again, so uc-agent.exe was
; in use, NSIS skipped it without an error and the old agent kept running. Now: close the
; window first, then stop the agent, then wait until nothing of ours runs from $INSTDIR
; (close-app.ps1, path-filtered, "taskkill /F" only as the last resort). Then every program
; file must be free; a file that cannot be replaced fails the install (exit code 2 in silent
; mode, Retry/Cancel otherwise). It never leaves a mix of old and new files without saying so.

; $PS = the 64-bit Windows PowerShell when there is one (this installer is 32-bit).
!macro UcFindPowerShell
  StrCpy $PS "$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe"
  ${If} ${FileExists} "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
    StrCpy $PS "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  ${EndIf}
!macroend

; Run one phase of close-app.ps1 (unpacked to $PLUGINSDIR). Leaves the exit code in $R0
; ("error" when PowerShell could not start). "$INSTDIR\." avoids a trailing backslash before
; the closing quote.
!macro UcCloseApp PHASE EXTRA
  nsExec::ExecToStack '"$PS" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\close-app.ps1" -Dir "$INSTDIR\." -Phase ${PHASE} ${EXTRA}'
  Pop $R0
  Pop $R1
  ${If} $R1 != ""
    DetailPrint "$R1"
  ${EndIf}
!macroend

; Add NAME to $Busy when it is there and cannot be opened for writing (a running program).
!macro UcBusyFile NAME
  ${If} ${FileExists} "$INSTDIR\${NAME}"
    ClearErrors
    FileOpen $R9 "$INSTDIR\${NAME}" a
    ${If} ${Errors}
      StrCpy $Busy "$Busy ${NAME};"
    ${Else}
      FileClose $R9
    ${EndIf}
  ${EndIf}
!macroend

; Remove an old program file before the copy, so a copy that fails leaves the file missing
; (checked after the copy) instead of silently old.
!macro UcDeleteOld NAME
  ${If} ${FileExists} "$INSTDIR\${NAME}"
    Delete "$INSTDIR\${NAME}"
    ${If} ${FileExists} "$INSTDIR\${NAME}"
      StrCpy $Busy "$Busy ${NAME};"
    ${EndIf}
  ${EndIf}
!macroend

; The new file must be there after the copy.
!macro UcMustExist NAME
  ${IfNot} ${FileExists} "$INSTDIR\${NAME}"
    StrCpy $Busy "$Busy ${NAME};"
  ${EndIf}
!macroend

; Stop with an error: exit code 2, and a message box unless silent.
!macro UcFail TEXT
  DetailPrint "${TEXT}"
  SetErrorLevel 2
  ${IfNot} ${Silent}
    MessageBox MB_OK|MB_ICONSTOP "${TEXT}"
  ${EndIf}
  Abort "${TEXT}"
!macroend

Function .onInit
  StrCpy $IsUpdate "0"
  StrCpy $AddFirewall "0"
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $IsUpdate "1"
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/ADDFIREWALL" $R1
  ${IfNot} ${Errors}
    StrCpy $AddFirewall "1"
  ${EndIf}
  ; Interactive installs always add the rule (one UAC prompt); silent ones only on request.
  ${IfNot} ${Silent}
    StrCpy $AddFirewall "1"
  ${EndIf}
FunctionEnd

Function OpenApp
  ${If} ${FileExists} "$INSTDIR\${UI_EXE}"
    Exec '"$INSTDIR\${UI_EXE}"'
  ${EndIf}
FunctionEnd

Section "Install"
  InitPluginsDir
  File "/oname=$PLUGINSDIR\close-app.ps1" "close-app.ps1"
  !insertmacro UcFindPowerShell
  StrCpy $Tries 0

  ; Stop what runs from an older install, so its files can be replaced (order: see above).
  close_app:
  ; 1. The window first, so nothing starts the agent again. (In an update the agent is
  ;    stopped already, and the helper's marker keeps the window from starting it.)
  !insertmacro UcCloseApp window ""
  ${If} $R0 != "0"
  ${AndIf} $R0 != "10"
    ; close-app.ps1 did not run: the old way for the window; the file check below decides.
    nsExec::Exec 'taskkill /IM "${UI_EXE}"'
    Pop $0
    Sleep 2000
  ${EndIf}
  ; 2. The agent (not for /UPDATE: the helper stopped it), 3. wait, end what is left.
  ${If} $IsUpdate == "0"
    !insertmacro UcCloseApp rest "-StopAgent"
    ${If} $R0 != "0"
    ${AndIf} $R0 != "10"
    ${AndIf} ${FileExists} "$INSTDIR\uc-agent.exe"
      nsExec::Exec '"$INSTDIR\uc-agent.exe" --stop'
      Pop $0
    ${EndIf}
  ${Else}
    !insertmacro UcCloseApp rest ""
  ${EndIf}

  ; 4. Every program file must be free, or nothing is replaced.
  StrCpy $Busy ""
  !insertmacro UcBusyFile "uc-agent.exe"
  !insertmacro UcBusyFile "uc.exe"
  !insertmacro UcBusyFile "uclip.exe"
  !insertmacro UcBusyFile "${UI_EXE}"
  ${If} $Busy == ""
    ; Free: remove the old program files (an older "uclip.exe" is made again by the hook).
    !insertmacro UcDeleteOld "uc-agent.exe"
    !insertmacro UcDeleteOld "uc.exe"
    !insertmacro UcDeleteOld "uclip.exe"
    !insertmacro UcDeleteOld "${UI_EXE}"
  ${EndIf}
  ${If} $Busy != ""
    IntOp $Tries $Tries + 1
    DetailPrint "In use, cannot be replaced:$Busy"
    ${If} ${Silent}
      ${If} $Tries < 3
        Sleep 1000
        Goto close_app
      ${EndIf}
    ${Else}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${NAME} is still running, so these files cannot be replaced:$Busy$\r$\n$\r$\nClose it and click Retry." IDRETRY close_app
    ${EndIf}
    !insertmacro UcFail "${NAME} is still running, so it was not updated. Close it and run the installer again."
  ${EndIf}

  SetOutPath "$INSTDIR"
  SetOverwrite on
  File /r "${SRC}\*.*"

  ; 5. Every new program file is there (the old ones were removed above).
  StrCpy $Busy ""
  !insertmacro UcMustExist "uc-agent.exe"
  !insertmacro UcMustExist "uc.exe"
  !if /FileExists "${SRC}\${UI_EXE}"
    !insertmacro UcMustExist "${UI_EXE}"
  !endif
  ${If} $Busy != ""
    !insertmacro UcFail "These files could not be written:$Busy The install is incomplete. Run the installer again."
  ${EndIf}
  File "/oname=icon.ico" "..\..\assets\icon.ico"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; "uc" on PATH, or "uclip" when a "uc" exists already (plan 3.2).
  nsExec::ExecToStack '"$INSTDIR\uc-agent.exe" --install-hook'
  Pop $0
  Pop $1
  DetailPrint "$1"

  ; The firewall rule (decision 2026-11-14): one UAC prompt now, none later.
  nsExec::Exec '"$INSTDIR\uc-agent.exe" --firewall check'
  Pop $0
  ${If} $0 == "0"
    DetailPrint "Firewall rule: already there."
  ${ElseIf} $AddFirewall == "1"
    DetailPrint "Adding the firewall rule. Windows asks for permission once."
    ExecWait '"$INSTDIR\uc-agent.exe" --firewall add --exe "$INSTDIR\uc-agent.exe"' $0
    ${If} $0 == "0"
      DetailPrint "Firewall rule: added. Your other devices can connect to this PC."
    ${ElseIf} $0 == "3"
      DetailPrint "Firewall rule: not added (you said no). This PC only connects out. Add it later: uc firewall add"
    ${Else}
      DetailPrint "Firewall rule: could not be added ($0). This PC only connects out. Try: uc firewall add"
    ${EndIf}
  ${Else}
    DetailPrint "Firewall rule: not added (silent install without /ADDFIREWALL)."
  ${EndIf}

  ${If} ${FileExists} "$INSTDIR\${UI_EXE}"
    CreateShortCut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\${UI_EXE}" "" "$INSTDIR\icon.ico"
  ${EndIf}

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Universal Clipboard contributors"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/Maxaubert/Universal-Clipboard"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"

  ; Silent installs (and updates) never open the app.
  ${If} $IsUpdate == "1"
    SetAutoClose true
  ${EndIf}
SectionEnd

Section "Uninstall"
  InitPluginsDir
  File "/oname=$PLUGINSDIR\close-app.ps1" "close-app.ps1"
  !insertmacro UcFindPowerShell
  ; 0. Close the window first: an open window would start the agent again after the hook
  ;    stopped it, and that agent would make the state folder and the Run value again.
  !insertmacro UcCloseApp window ""
  ${If} $R0 != "0"
  ${AndIf} $R0 != "10"
    nsExec::Exec 'taskkill /IM "${UI_EXE}"'
    Pop $0
  ${EndIf}
  ; 1. Tell the group (2 s), stop the agent, remove start at login, state and keys.
  ${If} ${FileExists} "$INSTDIR\uc-agent.exe"
    nsExec::ExecToStack '"$INSTDIR\uc-agent.exe" --uninstall-hook --dir "$INSTDIR"'
    Pop $0
    Pop $1
    DetailPrint "$1"
  ${EndIf}
  ; 2. Wait for what is left from this folder; end it as the last resort.
  !insertmacro UcCloseApp rest ""
  ${If} $R0 != "0"
  ${AndIf} $R0 != "10"
    nsExec::Exec 'taskkill /F /IM "uc-agent.exe"'
    Pop $0
  ${EndIf}

  ; 2. The firewall rule needs an administrator: offer it (plan 8.5), never force it.
  ${If} ${FileExists} "$INSTDIR\uc-agent.exe"
    nsExec::Exec '"$INSTDIR\uc-agent.exe" --firewall check'
    Pop $0
    ${If} $0 == "0"
      ${GetParameters} $R0
      ClearErrors
      ${GetOptions} $R0 "/REMOVEFIREWALL" $R1
      ${If} ${Errors}
        MessageBox MB_YESNO|MB_ICONQUESTION "Also remove the firewall rule? Windows asks for permission once." /SD IDNO IDNO skip_fw
      ${EndIf}
      ExecWait '"$INSTDIR\uc-agent.exe" --firewall remove' $0
      DetailPrint "Firewall rule removal: $0"
      skip_fw:
    ${EndIf}
  ${EndIf}

  ; 3. Our files only (never a recursive delete of the install folder).
  Delete "$INSTDIR\uc-agent.exe"
  Delete "$INSTDIR\uc.exe"
  Delete "$INSTDIR\uclip.exe"
  Delete "$INSTDIR\${UI_EXE}"
  Delete "$INSTDIR\cli-name.txt"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${NAME}.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
