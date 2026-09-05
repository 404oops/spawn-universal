; SPDX-License-Identifier: GPL-3.0-or-later
;
; Windows installer. Built by the release workflow, or by hand:
;
;   makensis -DVERSION=0.1.0 packaging\windows-installer.nsi
;
; It expects target\release\spawn-universal.exe and packaging\icons\icon.ico
; to exist already: cargo builds the first, make-icon.py draws the second.

!include "MUI2.nsh"
!include "LogicLib.nsh"
; GetSize, for the size shown in Add/Remove Programs.
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif

!define APPNAME "Spawn Universal"
!define SLUG "spawn-universal"
!define PUBLISHER "spawn-universal contributors"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${SLUG}"

Name "${APPNAME}"
OutFile "..\dist\${SLUG}-${VERSION}-windows-setup.exe"
Unicode true
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${SLUG}" "InstallDir"
; Writing to Program Files and HKLM both need it.
RequestExecutionLevel admin
SetCompressor /SOLID lzma

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "Configurator for SPAWN magnetic keyboards"
VIAddVersionKey "LegalCopyright" "${PUBLISHER}, GPL-3.0-or-later"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!define MUI_ABORTWARNING
!define MUI_ICON "icons\icon.ico"
!define MUI_UNICON "icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${SLUG}.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${APPNAME}"

!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  ; Only one program can hold a HID device at a time, so an upgrade that
  ; overwrites a running copy would fail on a locked file.
  ${If} ${FileExists} "$INSTDIR\${SLUG}.exe"
    ExecWait 'taskkill /F /IM ${SLUG}.exe' $0
    Sleep 500
  ${EndIf}

  SetOutPath "$INSTDIR"
  File "..\target\release\${SLUG}.exe"
  File "icons\icon.ico"
  File "..\LICENSE"
  File "..\README.md"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\${SLUG}" "InstallDir" "$INSTDIR"

  WriteRegStr HKLM "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTKEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKLM "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr HKLM "${UNINSTKEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  WriteRegStr HKLM "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKLM "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTKEY}" "NoRepair" 1

  ; So Add/Remove Programs shows a size rather than a blank.
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKLM "${UNINSTKEY}" "EstimatedSize" "$0"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\${SLUG}.exe" \
    "" "$INSTDIR\icon.ico"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk" "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ExecWait 'taskkill /F /IM ${SLUG}.exe' $0
  Sleep 500

  Delete "$INSTDIR\${SLUG}.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"

  DeleteRegKey HKLM "${UNINSTKEY}"
  DeleteRegKey HKLM "Software\${SLUG}"
SectionEnd
