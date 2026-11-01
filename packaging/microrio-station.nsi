; Windows installer for the MicroRIO driver station.
;
; Built on Linux with makensis (apt install nsis); see packaging/release.sh.
; Installs per user by default, so it needs no administrator rights: the
; station is a tool someone runs on a laptop, not a service.
;
;   VERSION and SRC are passed in with -D.

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef SRC
  !define SRC "."
!endif

!define NAME    "MicroRIO Driver Station"
!define EXE     "microrio-station.exe"
!define REGKEY  "Software\Microsoft\Windows\CurrentVersion\Uninstall\MicroRIO"

Name           "${NAME} ${VERSION}"
OutFile        "${OUT}"
Unicode        true
RequestExecutionLevel user
InstallDir     "$LOCALAPPDATA\Programs\MicroRIO"
InstallDirRegKey HKCU "Software\MicroRIO" "InstallDir"
ShowInstDetails show
ShowUninstDetails show
BrandingText   "MicroRIO ${VERSION}"

!include "MUI2.nsh"
!define MUI_ABORTWARNING
!define MUI_ICON   "${SRC}\microrio.ico"
!define MUI_UNICON "${SRC}\microrio.ico"

; Offer to open the station once it is installed.
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start the driver station"
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\RUNNING.md"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Open the running notes"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_LICENSE "${SRC}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Station" SecStation
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRC}\${EXE}"
  File "${SRC}\RUNNING.md"
  File "${SRC}\MicroRIO-design.md"
  File "${SRC}\LICENSE.txt"
  File "${SRC}\microrio.ico"

  ; The Arduino library ships alongside, so a board can be set up from the
  ; same install rather than hunting for a matching version of it.
  SetOutPath "$INSTDIR\arduino"
  File /nonfatal "${SRC}\arduino\*.zip"
  SetOutPath "$INSTDIR"

  CreateDirectory "$SMPROGRAMS\MicroRIO"
  CreateShortcut "$SMPROGRAMS\MicroRIO\${NAME}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\microrio.ico"
  CreateShortcut "$SMPROGRAMS\MicroRIO\Chart.lnk" "$INSTDIR\${EXE}" "--tab chart" "$INSTDIR\microrio.ico"
  CreateShortcut "$SMPROGRAMS\MicroRIO\Check a controller.lnk" "$INSTDIR\${EXE}" "--tab joystick" "$INSTDIR\microrio.ico"
  CreateShortcut "$SMPROGRAMS\MicroRIO\Uninstall.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "Software\MicroRIO" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${REGKEY}" "DisplayName"     "${NAME}"
  WriteRegStr HKCU "${REGKEY}" "DisplayVersion"  "${VERSION}"
  WriteRegStr HKCU "${REGKEY}" "DisplayIcon"     "$INSTDIR\microrio.ico"
  WriteRegStr HKCU "${REGKEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${REGKEY}" "Publisher"       "MicroRIO"
  WriteRegDWORD HKCU "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${REGKEY}" "NoRepair" 1
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${NAME}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\microrio.ico"
SectionEnd

LangString DESC_SecStation ${LANG_ENGLISH} "The driver station, its notes, and the Arduino library zip."
LangString DESC_SecDesktop ${LANG_ENGLISH} "A shortcut on the desktop as well as the Start menu."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStation} $(DESC_SecStation)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_SecDesktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\RUNNING.md"
  Delete "$INSTDIR\MicroRIO-design.md"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\microrio.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /r "$INSTDIR\arduino"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\MicroRIO\*.lnk"
  RMDir "$SMPROGRAMS\MicroRIO"
  Delete "$DESKTOP\${NAME}.lnk"

  DeleteRegKey HKCU "${REGKEY}"
  DeleteRegKey HKCU "Software\MicroRIO"

  ; Profiles and logs live in %APPDATA% and are the operator's own work.
  ; They are deliberately left behind.
SectionEnd
