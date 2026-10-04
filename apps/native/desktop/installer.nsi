; The updater replaces app/ atomically. Keep installer metadata outside it.
Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
Name "Caper"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Caper"
RequestExecutionLevel user
SetCompressor /SOLID lzma
AutoCloseWindow true
ShowInstDetails nevershow
Icon "resources/caper.ico"
UninstallIcon "resources/caper.ico"
!define MUI_ABORTWARNING
; Opening Setup immediately installs; no Welcome, Next, or Finish clicks.
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "Caper requires 64-bit Windows."
    Abort
  ${EndIf}
  SetShellVarContext current
  SetRegView 64
FunctionEnd

Function .onInstSuccess
  ; Automated /S installs must not open a GUI or initialize audio devices.
  IfSilent done
  Exec '"$INSTDIR\app\Caper.exe"'
  done:
FunctionEnd

Section "Caper"
  SetOutPath "$INSTDIR\app"
  File "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Caper.lnk" "$INSTDIR\app\Caper.exe"
  CreateShortcut "$DESKTOP\Caper.lnk" "$INSTDIR\app\Caper.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "DisplayName" "Caper"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "Publisher" "Caper"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "DisplayIcon" "$INSTDIR\app\Caper.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  SetRegView 64
  ; A running Windows executable cannot be deleted. Stop before removing its
  ; dependencies if the app is still open. Never touch credentials/settings.
  Delete "$INSTDIR\app\Caper.exe"
  IfFileExists "$INSTDIR\app\Caper.exe" 0 +3
    MessageBox MB_OK|MB_ICONSTOP "Close Caper before uninstalling, then try again." /SD IDOK
    Abort
  ; Remove only this install's opt-in login entry, not a different portable copy.
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Caper"
  StrCmp $0 '"$INSTDIR\app\Caper.exe"' 0 +2
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Caper"
  RMDir /r "$INSTDIR\app"
  Delete "$SMPROGRAMS\Caper.lnk"
  Delete "$DESKTOP\Caper.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
