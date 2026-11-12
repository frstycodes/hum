; Hum installer (NSIS 3), set up like Stash's. Per-user: installs to
; %LOCALAPPDATA%\Programs\Hum without asking for admin rights. Built by
; installer\build.ps1, which passes /DVERSION, /DEXE (the built hum.exe) and /DICON.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "Pass /DVERSION=x.y.z (installer\build.ps1 does this)"
!endif

!define APP "Hum"
!define EXE_NAME "hum.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${APP}"
OutFile "..\dist\Hum-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "CompanyName" "Sandesh Pandey"
VIAddVersionKey "LegalCopyright" "Copyright 2026 Sandesh Pandey"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_WELCOMEPAGE_TEXT "Hum turns your phone into your PC's microphone over Wi-Fi.$\r$\n$\r$\nIt sends the sound to VB-Cable, a free virtual audio driver. If you don't have it yet, Hum offers to install it the first time it runs.$\r$\n$\r$\nClick Next to continue."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE_NAME}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP} now"
!define MUI_FINISHPAGE_TEXT "${APP} is installed.$\r$\n$\r$\nThe first time it runs, Windows asks whether ${APP} may use your network: allow private networks so your phone can reach it.$\r$\n$\r$\nClosing the window keeps ${APP} running in the tray (next to the clock). Quit it from the tray icon's menu."

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; A running copy (maybe hidden in the tray) holds the exe open; close it first.
!macro StopHum
  nsExec::Exec 'taskkill /F /IM ${EXE_NAME}'
  Pop $0
  Sleep 300
!macroend

; ...and wait until the file is free: up to 15 seconds.
Function WaitForExe
  StrCpy $1 0
  loop:
    IfFileExists "$INSTDIR\${EXE_NAME}" 0 done
    ClearErrors
    FileOpen $2 "$INSTDIR\${EXE_NAME}" a
    IfErrors 0 free
    IntOp $1 $1 + 1
    IntCmp $1 60 done
    nsExec::Exec 'taskkill /F /IM ${EXE_NAME}'
    Pop $0
    Sleep 250
    Goto loop
  free:
    FileClose $2
  done:
FunctionEnd

Section "${APP}" SecApp
  SectionIn RO
  !insertmacro StopHum
  Call WaitForExe
  SetOutPath "$INSTDIR"
  File "/oname=${EXE_NAME}" "${EXE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ; keep a "start when I sign in" choice pointing at this copy
  ReadRegStr $0 HKCU "${RUN_KEY}" "${APP}"
  ${If} $0 != ""
    WriteRegStr HKCU "${RUN_KEY}" "${APP}" '"$INSTDIR\${EXE_NAME}"'
  ${EndIf}

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE_NAME}"

  ; Apps & features entry
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Sandesh Pandey"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE_NAME}"
SectionEnd

; Off unless ticked: Hum then waits in the tray from sign-in.
Section /o "Start ${APP} when I sign in" SecLogin
  WriteRegStr HKCU "${RUN_KEY}" "${APP}" '"$INSTDIR\${EXE_NAME}"'
SectionEnd

Section "Uninstall"
  !insertmacro StopHum
  Delete "$INSTDIR\${EXE_NAME}"
  ; the phone page's certificate, made on first run
  Delete "$INSTDIR\hum-cert.der"
  Delete "$INSTDIR\hum-key.der"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "${APP}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\${APP}"
  ; VB-Cable is its own driver with its own uninstaller; it stays.
SectionEnd
