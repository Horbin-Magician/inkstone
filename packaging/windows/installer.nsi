Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
Name "墨砚"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Inkstone"
InstallDirRegKey HKCU "Software\Inkstone" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Inkstone"
VIAddVersionKey "FileDescription" "Inkstone Installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Licensed under Apache-2.0"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Inkstone requires 64-bit Windows."
    Abort
  ${EndIf}
FunctionEnd

Section "Inkstone"
  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\墨砚.lnk" "$INSTDIR\inkstone.exe"
  WriteRegStr HKCU "Software\Inkstone" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "DisplayName" "墨砚"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "DisplayIcon" "$INSTDIR\inkstone.exe"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\墨砚.lnk"
  Delete "$INSTDIR\inkstone.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\GUIDE.md"
  Delete "$INSTDIR\resources.json"
  Delete "$INSTDIR\licenses\KaTeX-fonts-NOTICE.txt"
  Delete "$INSTDIR\licenses\SIL-OFL-1.1.txt"
  RMDir "$INSTDIR\licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone"
  DeleteRegKey HKCU "Software\Inkstone"
SectionEnd
