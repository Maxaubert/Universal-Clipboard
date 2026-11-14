; Universal Clipboard: per-user Windows installer (plan 2.1, 8.1, 8.5). No admin prompt.
;
; Build: makensis /DVERSION=0.1.0 /DSRC=<folder with the exes> /DOUT=<setup.exe> installer.nsi
; (packaging/windows/build.ps1 does this.)
;
; Switches:
;   /S                 silent
;   /UPDATE            used by the updater: do not stop the agent (the helper did), do not
;                      start anything at the end
;   /REMOVEFIREWALL    uninstaller only: also remove the firewall rule without asking
;   /D=<folder>        install folder (last, no quotes)
;
; The logic lives in uc-agent (tested in Rust): --install-hook puts "uc" on PATH (or "uclip"
; when a "uc" already exists), --uninstall-hook tells the group, stops the agent and removes
; start at login, the state folder and the keys. The firewall rule needs an administrator, so
; the uninstaller only offers it (one UAC prompt).

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
!define MUI_WELCOMEPAGE_TEXT "Copy on one device, paste on another.$\r$\n$\r$\nThis installs ${NAME} for you only. It needs no administrator.$\r$\n$\r$\nNo account, no server. Your clipboard only moves between your own devices on your local network.$\r$\n$\r$\nPrivacy: ${PRIVACY}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open ${NAME}"
!define MUI_FINISHPAGE_RUN_FUNCTION OpenApp
!define MUI_FINISHPAGE_TEXT "${NAME} is installed.$\r$\n$\r$\nWithout the window, open a new terminal and run: uc setup"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Var IsUpdate

Function .onInit
  StrCpy $IsUpdate "0"
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $IsUpdate "1"
  ${EndIf}
FunctionEnd

Function OpenApp
  ${If} ${FileExists} "$INSTDIR\${UI_EXE}"
    Exec '"$INSTDIR\${UI_EXE}"'
  ${EndIf}
FunctionEnd

Section "Install"
  ; Stop what runs from an older install, so its files can be replaced.
  ${If} $IsUpdate == "0"
  ${AndIf} ${FileExists} "$INSTDIR\uc-agent.exe"
    nsExec::Exec '"$INSTDIR\uc-agent.exe" --stop'
    Pop $0
  ${EndIf}
  nsExec::Exec 'taskkill /IM "${UI_EXE}"'
  Pop $0

  SetOutPath "$INSTDIR"
  ; An older "uclip.exe" (renamed "uc.exe") is replaced by the hook below.
  File /r "${SRC}\*.*"
  File "/oname=icon.ico" "..\..\assets\icon.ico"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; "uc" on PATH, or "uclip" when a "uc" exists already (plan 3.2).
  nsExec::ExecToStack '"$INSTDIR\uc-agent.exe" --install-hook'
  Pop $0
  Pop $1
  DetailPrint "$1"

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
  ; 1. Tell the group (2 s), stop the agent, remove start at login, state and keys.
  ${If} ${FileExists} "$INSTDIR\uc-agent.exe"
    nsExec::ExecToStack '"$INSTDIR\uc-agent.exe" --uninstall-hook --dir "$INSTDIR"'
    Pop $0
    Pop $1
    DetailPrint "$1"
  ${EndIf}
  nsExec::Exec 'taskkill /IM "${UI_EXE}"'
  Pop $0
  nsExec::Exec 'taskkill /F /IM "uc-agent.exe"'
  Pop $0

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
