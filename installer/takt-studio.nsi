; Installateur Windows de Takt Studio, pour l'utilisateur (sans droits administrateur).
;
; Installe dans %LOCALAPPDATA%\Programs\Takt Studio : l'IDE, le compilateur en ligne de commande
; (taktc), le projet d'exemple et les licences. Associe les fichiers .takt à Takt Studio,
; crée les raccourcis et l'entrée « Applications installées » de Windows.
;
; Options :
;   /S       installation silencieuse
;   /UPDATE  mise à jour lancée depuis l'appli : garde les raccourcis, relance l'appli à la fin
;
; Construit par tools/installeur.sh :
;   makensis /DVERSION=0.2.0 /DSRC=<dossier des fichiers> /DSORTIE=<setup.exe> takt-studio.nsi

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!define NOM "Takt Studio"
!define BINAIRE "takt-studio.exe"
!define EDITEUR "Pierre Bayle"
!define SITE "https://github.com/ideaton/takt"
!define CLE_DESINSTALL "Software\Microsoft\Windows\CurrentVersion\Uninstall\${NOM}"
!define TYPE_FICHIER "TaktStudio.Projet"

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

Name "${NOM}"
OutFile "${SORTIE}"
InstallDir "$LOCALAPPDATA\Programs\${NOM}"
InstallDirRegKey HKCU "${CLE_DESINSTALL}" "InstallLocation"
BrandingText "${NOM} ${VERSION}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${NOM}"
VIAddVersionKey "FileDescription" "Installateur de ${NOM}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${EDITEUR}"
VIAddVersionKey "LegalCopyright" "GPL-3.0"

!define MUI_ICON "..\crates\takt-ide\assets\takt.ico"
!define MUI_UNICON "..\crates\takt-ide\assets\takt.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Installation de ${NOM} ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "Takt Studio est un atelier de programmation d'automates façon TIA Portal, pour les cartes RP2350 (Raspberry Pi Pico 2…), avec un temps de cycle garanti.$\r$\n$\r$\nLangages : CONT (ladder), SCL, GRAPH et BASIC. Simulateur intégré.$\r$\n$\r$\nAucun droit administrateur n'est nécessaire."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${BINAIRE}"
!define MUI_FINISHPAGE_RUN_TEXT "Lancer ${NOM}"
!define MUI_FINISHPAGE_LINK "Le projet sur GitHub"
!define MUI_FINISHPAGE_LINK_LOCATION "${SITE}"

Var MiseAJour

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SRC}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "French"

Function .onInit
  ${GetParameters} $0
  StrCpy $MiseAJour 0
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  ${IfNot} ${Errors}
    StrCpy $MiseAJour 1
    SetSilent silent
  ${EndIf}
FunctionEnd

!macro FermerAppli
  ; L'appli doit être fermée pour remplacer l'exécutable
  nsExec::Exec 'taskkill /IM ${BINAIRE} /F'
  Pop $0
  Sleep 600
!macroend

Section "Takt Studio"
  !insertmacro FermerAppli
  SetOutPath "$INSTDIR"
  ; Jusqu'à 10 essais : Windows libère parfois l'exécutable avec un peu de retard
  StrCpy $2 0
  copie:
  ClearErrors
  File "${SRC}\${BINAIRE}"
  ${If} ${Errors}
    IntOp $2 $2 + 1
    ${If} $2 < 10
      Sleep 500
      Goto copie
    ${EndIf}
    MessageBox MB_ICONSTOP "Impossible de remplacer ${BINAIRE} : fermez Takt Studio et relancez l'installation."
    Abort
  ${EndIf}
  File "${SRC}\taktc.exe"
  File "${SRC}\LICENSE.txt"
  File "${SRC}\LISEZMOI.txt"
  SetOutPath "$INSTDIR\licences"
  File "${SRC}\licences\*.*"
  SetOutPath "$INSTDIR\exemples"
  File "${SRC}\exemples\*.takt"

  ; Copie de l'exemple dans les Documents, sans écraser une version modifiée
  SetOverwrite off
  SetOutPath "$DOCUMENTS\Takt"
  File "${SRC}\exemples\convoyeur.takt"
  SetOverwrite on
  SetOutPath "$INSTDIR"

  WriteUninstaller "$INSTDIR\desinstaller.exe"

  ; Entrée « Applications installées »
  WriteRegStr HKCU "${CLE_DESINSTALL}" "DisplayName" "${NOM}"
  WriteRegStr HKCU "${CLE_DESINSTALL}" "DisplayIcon" '"$INSTDIR\${BINAIRE}"'
  WriteRegStr HKCU "${CLE_DESINSTALL}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${CLE_DESINSTALL}" "Publisher" "${EDITEUR}"
  WriteRegStr HKCU "${CLE_DESINSTALL}" "URLInfoAbout" "${SITE}"
  WriteRegStr HKCU "${CLE_DESINSTALL}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${CLE_DESINSTALL}" "UninstallString" '"$INSTDIR\desinstaller.exe"'
  WriteRegStr HKCU "${CLE_DESINSTALL}" "QuietUninstallString" '"$INSTDIR\desinstaller.exe" /S'
  WriteRegDWORD HKCU "${CLE_DESINSTALL}" "NoModify" 1
  WriteRegDWORD HKCU "${CLE_DESINSTALL}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${CLE_DESINSTALL}" "EstimatedSize" $0

  ; Les fichiers .takt s'ouvrent avec Takt Studio
  WriteRegStr HKCU "Software\Classes\.takt" "" "${TYPE_FICHIER}"
  WriteRegStr HKCU "Software\Classes\${TYPE_FICHIER}" "" "Projet Takt"
  WriteRegStr HKCU "Software\Classes\${TYPE_FICHIER}\DefaultIcon" "" '"$INSTDIR\${BINAIRE}",0'
  WriteRegStr HKCU "Software\Classes\${TYPE_FICHIER}\shell\open\command" "" '"$INSTDIR\${BINAIRE}" "%1"'
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

  ; Raccourcis : menu Démarrer toujours, bureau seulement à la première installation
  CreateShortcut "$SMPROGRAMS\${NOM}.lnk" "$INSTDIR\${BINAIRE}"
  ${If} $MiseAJour = 0
    CreateShortcut "$DESKTOP\${NOM}.lnk" "$INSTDIR\${BINAIRE}"
  ${Else}
    Exec '"$INSTDIR\${BINAIRE}"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  !insertmacro FermerAppli
  Delete "$INSTDIR\${BINAIRE}"
  Delete "$INSTDIR\taktc.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\LISEZMOI.txt"
  RMDir /r "$INSTDIR\licences"
  RMDir /r "$INSTDIR\exemples"
  Delete "$INSTDIR\desinstaller.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${NOM}.lnk"
  Delete "$DESKTOP\${NOM}.lnk"
  DeleteRegKey HKCU "${CLE_DESINSTALL}"
  ; L'association n'est retirée que si elle pointe encore vers Takt Studio
  ReadRegStr $0 HKCU "Software\Classes\.takt" ""
  ${If} $0 == "${TYPE_FICHIER}"
    DeleteRegKey HKCU "Software\Classes\.takt"
  ${EndIf}
  DeleteRegKey HKCU "Software\Classes\${TYPE_FICHIER}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Les projets de l'utilisateur (Documents\Takt) et ses réglages sont conservés.
SectionEnd
