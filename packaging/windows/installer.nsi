; Chewvy installer for Windows. Built by build-installer.sh, which passes
; VERSION, STAGE (the directory to install), OUTFILE and FILELIST (generated
; install and uninstall file lists) on the makensis command line.
;
; Per-user install: no administrator rights, files under the user's own
; Programs folder, registry entries under HKCU only.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include "MUI2.nsh"

!define APP "Chewvy"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Chewvy"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Chewvy"
InstallDirRegKey HKCU "Software\Chewvy" "InstallDir"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "GPL-3.0-or-later"

!define MUI_ICON "chewvy.ico"
!define MUI_UNICON "chewvy.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\chewvy.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP}"

!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!include "${FILELIST}"

Section "Install"
  SetOutPath "$INSTDIR"
  !insertmacro InstallFiles
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\chewvy.exe" "" "$INSTDIR\chewvy.ico"

  WriteRegStr HKCU "Software\Chewvy" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Chewvy contributors"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/Ch3w3y/chewvy"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\chewvy.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

  ; The bundled mpv imports the Vulkan loader, which only a graphics driver
  ; installs. Say so now rather than let the first start fail without a reason.
  IfFileExists "$WINDIR\Sysnative\vulkan-1.dll" vulkan_found
  IfFileExists "$SYSDIR\vulkan-1.dll" vulkan_found
  IfSilent vulkan_found
  MessageBox MB_OK|MB_ICONEXCLAMATION "No Vulkan runtime (vulkan-1.dll) was found on this PC. ${APP} needs one to start: install or update your graphics driver."
  vulkan_found:
SectionEnd

Section "Uninstall"
  ; Only the files this installer put there are removed, so a folder the user
  ; shares with something else is left alone. Settings and sign-ins under
  ; %APPDATA%\chewvy are kept.
  !insertmacro UninstallFiles
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\Chewvy"
SectionEnd
