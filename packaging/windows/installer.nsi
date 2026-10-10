Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
Name "墨砚"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Inkstone"
InstallDirRegKey HKCU "Software\Inkstone" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true
BrandingText "Inkstone · ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow
!define MUI_ABORTWARNING
!define MUI_TEXTCOLOR "292B2D"
!define MUI_BGCOLOR "FFFFFF"
!define MUI_WELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\assets\sidebar.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "${MUI_WELCOMEFINISHPAGE_BITMAP}"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "${__FILEDIR__}\assets\header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "${MUI_HEADERIMAGE_BITMAP}"
!define MUI_WELCOMEPAGE_TITLE "$(WelcomeTitle)"
!define MUI_WELCOMEPAGE_TEXT "$(WelcomeText)"
!define MUI_FINISHPAGE_TITLE "$(FinishTitle)"
!define MUI_FINISHPAGE_TEXT "$(FinishText)"
!define MUI_FINISHPAGE_RUN "$INSTDIR\inkstone.exe"
!define MUI_FINISHPAGE_RUN_TEXT "$(LaunchText)"
!define MUI_ICON "${__FILEDIR__}\..\..\crates\inkstone-desktop\assets\inkstone.ico"
!define MUI_UNICON "${MUI_ICON}"
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

LangString WelcomeTitle ${LANG_SIMPCHINESE} "安装墨砚"
LangString WelcomeTitle ${LANG_ENGLISH} "Install Inkstone"
LangString WelcomeText ${LANG_SIMPCHINESE} "让想法落在纸上。$\r$\n$\r$\n墨砚将在这台电脑上为你安装，无需管理员权限。$\r$\n$\r$\n点击“下一步”开始。"
LangString WelcomeText ${LANG_ENGLISH} "A quiet place for your ideas.$\r$\n$\r$\nInstall Inkstone for your account. No administrator access needed.$\r$\n$\r$\nChoose Next to begin."
LangString FinishTitle ${LANG_SIMPCHINESE} "准备好，开始记录"
LangString FinishTitle ${LANG_ENGLISH} "Ready for your ideas"
LangString FinishText ${LANG_SIMPCHINESE} "墨砚已安装完成。$\r$\n$\r$\n你也可以从桌面或开始菜单打开墨砚。"
LangString FinishText ${LANG_ENGLISH} "Inkstone is installed.$\r$\n$\r$\nYou can also open it from your desktop or Start menu."
LangString LaunchText ${LANG_SIMPCHINESE} "打开墨砚"
LangString LaunchText ${LANG_ENGLISH} "Open Inkstone"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Inkstone requires 64-bit Windows."
    Abort
  ${EndIf}
FunctionEnd

Section "Inkstone"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\墨砚.lnk" "$INSTDIR\inkstone.exe" "" "$INSTDIR\inkstone.exe" 0
  CreateShortcut "$DESKTOP\墨砚.lnk" "$INSTDIR\inkstone.exe" "" "$INSTDIR\inkstone.exe" 0
  WriteRegStr HKCU "Software\Inkstone" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "DisplayName" "墨砚"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "DisplayIcon" '$\"$INSTDIR\inkstone.exe$\",0'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Inkstone" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$SMPROGRAMS\墨砚.lnk"
  Delete "$DESKTOP\墨砚.lnk"
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
