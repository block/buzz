!macro BUZZ_STOP_PROCESS image_name
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /T /IM "${image_name}"'
  Pop $0
!macroend

!macro BUZZ_STOP_RUNNING_PROCESSES
  !insertmacro BUZZ_STOP_PROCESS "buzz-desktop.exe"
  !insertmacro BUZZ_STOP_PROCESS "buzz-acp.exe"
  !insertmacro BUZZ_STOP_PROCESS "buzz-agent.exe"
  !insertmacro BUZZ_STOP_PROCESS "buzz-dev-mcp.exe"
  !insertmacro BUZZ_STOP_PROCESS "buzz.exe"
  !insertmacro BUZZ_STOP_PROCESS "git-credential-nostr.exe"
  Sleep 500
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro BUZZ_STOP_RUNNING_PROCESSES
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro BUZZ_STOP_RUNNING_PROCESSES
!macroend
