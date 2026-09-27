; Installer hooks for the Hardwave DAW setup.
;
; The DAW used to install for all users, which meant every install and every
; update asked for administrator rights. Nothing in the app needs them: it
; writes its settings and projects under the user's own folders. It installs
; per user now, into %LOCALAPPDATA%, so installing and updating no longer
; prompt at all.
;
; That leaves people who installed the old way. Their copy lives in Program
; Files and is registered machine-wide, and the new installer would simply
; sit beside it: two entries in the Start menu, two in Apps & features, and
; an updater pointing at whichever was launched. This hook removes the old
; one first. Removing it needs administrator rights once, so it asks, and if
; the person says no the install carries on and they keep both copies rather
; than losing the installer entirely.

!macro NSIS_HOOK_PREINSTALL
  ; An old machine-wide install registers itself under HKLM with the same
  ; key this installer uses under HKCU.
  ReadRegStr $R7 HKLM "${UNINSTKEY}" "UninstallString"
  ${If} $R7 != ""
    ${If} ${Silent}
      ; An update runs silently and cannot show a prompt. Try the old
      ; uninstaller quietly; on a machine where that needs elevation it
      ; fails and both copies remain, which the next manual install fixes.
      ExecWait '$R7 /S'
    ${Else}
      MessageBox MB_YESNO|MB_ICONQUESTION \
        "An older copy of ${PRODUCTNAME} is installed for all users.$\r$\n$\r$\n\
         Remove it first? Windows will ask for administrator rights once. \
         Your projects and settings are not touched.$\r$\n$\r$\n\
         Choosing No installs this copy alongside the old one." \
        IDNO hook_keep_old
      ExecWait '$R7 /S'
      hook_keep_old:
    ${EndIf}
  ${EndIf}
!macroend
