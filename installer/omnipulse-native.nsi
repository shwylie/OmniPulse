; OmniPulse's installer, for the native app (crates/app): one program, no WebView. It installs where
; the web app's installer does (this user's %LOCALAPPDATA%\OmniPulse) and takes over its uninstall
; entry, its omnipulse:// links and its Start menu shortcut, so it updates the web app in place (your
; data in %APPDATA%\sc-client is untouched: the native app reads the web app's from it).
;
; Built by scripts/release-native.mjs:  makensis /DVERSION=0.3.1 /DOUT=<setup.exe> installer\omnipulse-native.nsi
; /P (passive, as updates run it): its progress only, nothing to click. It starts OmniPulse when it's done.

Unicode true
!ifndef VERSION
  !define VERSION "0.3.1"
!endif
!ifndef OUT
  !define OUT "..\target\release\bundle\native\OmniPulse_${VERSION}_x64-setup.exe"
!endif
!define SRC "..\target\release"
!define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniPulse"

Name "OmniPulse"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\OmniPulse"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "OmniPulse ${VERSION}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "OmniPulse"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "OmniPulse installer"
VIAddVersionKey "LegalCopyright" "OmniPulse"

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!define MUI_ICON "..\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\src-tauri\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\OmniPulse.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start OmniPulse"
!insertmacro MUI_PAGE_INSTFILES
; (passive: no finish page; OmniPulse starts by itself)
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Var Passive

Function .onInit
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/P" $R1
  ${IfNot} ${Errors}
    StrCpy $Passive "1"
    SetAutoClose true
  ${EndIf}
FunctionEnd

Function SkipIfPassive
  ${If} $Passive == "1"
    Abort
  ${EndIf}
FunctionEnd

Section "OmniPulse"
  ; a running OmniPulse (the web app, or this one) closes first, so its files can be replaced
  nsExec::Exec 'taskkill /IM OmniPulse.exe /F'
  nsExec::Exec 'taskkill /IM omnipulse-offset.exe /F'
  Sleep 800

  SetOutPath "$INSTDIR"
  File "/oname=OmniPulse.exe" "${SRC}\OmniPulse-native.exe"
  File "${SRC}\omnipulse-offset.exe"
  ; the plugins that come with OmniPulse (installed from here when it starts)
  SetOutPath "$INSTDIR\plugins\internet-archive"
  File /r "..\plugins\internet-archive\*.*"
  SetOutPath "$INSTDIR"
  ; the web app's files this one doesn't need (it has its icons built in, and no separate input program)
  Delete "$INSTDIR\omnipulse-input.exe"
  Delete "$INSTDIR\offset-omnipulse.exe"
  RMDir /r "$INSTDIR\icons"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${UNINST}" "DisplayName" "OmniPulse"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" '"$INSTDIR\OmniPulse.exe"'
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "Publisher" "scclient"
  WriteRegStr HKCU "${UNINST}" "InstallLocation" '"$INSTDIR"'
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST}" "MainBinaryName" "OmniPulse.exe"
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST}" "EstimatedSize" $0

  ; omnipulse:// links (Jams, Listen Along, collaborative playlists) open it
  WriteRegStr HKCU "Software\Classes\omnipulse" "" "URL:OmniPulse"
  WriteRegStr HKCU "Software\Classes\omnipulse" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\omnipulse\DefaultIcon" "" '"$INSTDIR\OmniPulse.exe",0'
  WriteRegStr HKCU "Software\Classes\omnipulse\shell\open\command" "" '"$INSTDIR\OmniPulse.exe" "%1"'

  CreateShortcut "$SMPROGRAMS\OmniPulse.lnk" "$INSTDIR\OmniPulse.exe"

  ; passive (an update): started again straight away
  ${If} $Passive == "1"
    Exec '"$INSTDIR\OmniPulse.exe"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM OmniPulse.exe /F'
  nsExec::Exec 'taskkill /IM omnipulse-offset.exe /F'
  Sleep 500
  Delete "$INSTDIR\OmniPulse.exe"
  Delete "$INSTDIR\omnipulse-offset.exe"
  RMDir /r "$INSTDIR\plugins"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\OmniPulse.lnk"
  DeleteRegKey HKCU "${UNINST}"
  DeleteRegKey HKCU "Software\Classes\omnipulse"
  ; (your music, settings and sign-ins in %APPDATA%\sc-client stay: reinstalling finds them)
SectionEnd
