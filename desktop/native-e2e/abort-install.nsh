; A real signed NSIS payload that fails before changing the installed files.
!macro NSIS_HOOK_PREINSTALL
  Abort "Deliberate native updater regression failure"
!macroend
