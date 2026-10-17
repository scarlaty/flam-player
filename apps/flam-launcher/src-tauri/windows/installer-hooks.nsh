; Hooks NSIS de Tauri 2 (bundle.windows.nsis.installerHooks), voir #180.
;
; Probleme : un fichier present dans la version N-1 et absent de la version N survivait dans le dossier
; d'installation apres une mise a jour (l'installateur ne fait que copier les fichiers de N).
;
; Solution : avant la copie, on lance le desinstallateur de la version precedente, deja installe dans
; $INSTDIR. Il connait exactement la liste des fichiers que N-1 a installes (ressources, binaire,
; uninstall.exe) et ne supprime QUE ceux-la : un fichier qui n'est pas dans cette liste (dossier
; d'installation partage, fichier ajoute par l'utilisateur) n'est jamais touche, et le dossier lui-meme
; n'est retire que s'il est vide. Aucun `RMDir /r` sur $INSTDIR. Cela fonctionne aussi pour une N-1
; anterieure a ce hook (aucun manifeste a ecrire a l'installation).
;
; `/UPDATE` : le desinstallateur conserve raccourcis, valeur de demarrage automatique et donnees
; utilisateur (hors $INSTDIR, de toute facon jamais supprimees sans la case "supprimer les donnees").
; `/S` : silencieux. `_?=` (toujours en dernier, sans guillemets) : execution sur place, synchrone ;
; uninstall.exe n'est donc pas supprime, l'installateur le reecrit juste apres.
; Tout echec est non bloquant : on retombe sur le comportement precedent (copie par-dessus).

!macro NSIS_HOOK_PREINSTALL
  ${If} ${FileExists} "$INSTDIR\uninstall.exe"
    DetailPrint "Nettoyage de la version precedente"
    ClearErrors
    ExecWait '"$INSTDIR\uninstall.exe" /S /UPDATE _?=$INSTDIR' $R9
    ${If} ${Errors}
    ${OrIf} $R9 <> 0
      DetailPrint "Nettoyage de la version precedente impossible (code $R9) : mise a jour par-dessus"
    ${EndIf}
    ; Le desinstallateur peut avoir retire un dossier devenu vide : le recreer avant la copie.
    CreateDirectory "$INSTDIR"
    SetOutPath "$INSTDIR"
  ${EndIf}
!macroend
