; BaseLens installer for Windows (NSIS 3).
;   makensis -DVERSION=0.1.0 -DEXE=path/to/BaseLens.exe -DOUT=dist/BaseLens-0.1.0-windows-x64-setup.exe packaging/windows/installer.nsi
; Installs for the current user (no admin prompt), with Start menu and
; desktop shortcuts, the BaseLens icon, and an uninstaller in "Apps".
Unicode true
SetCompressor /SOLID lzma

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!define APP "BaseLens"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\BaseLens"

Name "${APP} ${VERSION}"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\BaseLens"
InstallDirRegKey HKCU "Software\BaseLens" "InstallDir"
BrandingText "${APP} ${VERSION}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License"

!include "MUI2.nsh"
!define MUI_ICON "..\..\assets\baselens.ico"
!define MUI_UNICON "..\..\assets\baselens.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\BaseLens.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open BaseLens"
!insertmacro MUI_PAGE_LICENSE "..\..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "PortugueseBR"

Section "BaseLens"
  SetOutPath "$INSTDIR"
  File "/oname=BaseLens.exe" "${EXE}"
  File "..\..\assets\baselens.ico"
  File "..\..\LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortCut "$SMPROGRAMS\BaseLens.lnk" "$INSTDIR\BaseLens.exe" "" "$INSTDIR\baselens.ico" 0
  CreateShortCut "$DESKTOP\BaseLens.lnk" "$INSTDIR\BaseLens.exe" "" "$INSTDIR\baselens.ico" 0
  WriteRegStr HKCU "Software\BaseLens" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "BaseLens"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\baselens.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\BaseLens.lnk"
  Delete "$DESKTOP\BaseLens.lnk"
  Delete "$INSTDIR\BaseLens.exe"
  Delete "$INSTDIR\baselens.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\BaseLens"
  ; settings and saved connections in %APPDATA%\baselens stay
SectionEnd
