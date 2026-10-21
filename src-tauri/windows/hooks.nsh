; (21/10) Hooks del instalador NSIS de SimFleet (bundle.windows.nsis.installerHooks).
;
; Tras instalar o actualizar, avisa a Windows de que los iconos cambiaron. La
; barra de tareas agrupa la app bajo el acceso del menú Inicio (mismo
; AppUserModelID) y le pone el icono de ese acceso, que Windows saca de su
; caché: sin esto, al actualizar encima de una versión con otro logo seguía
; saliendo el icono viejo.

!macro NSIS_HOOK_POSTINSTALL
  ; SHCNE_ASSOCCHANGED (0x08000000) + SHCNF_IDLIST (0): refresca los iconos del Explorador.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Rehace la caché de iconos. El instalador es de 32 bits: en Windows de 64 bits
  ; ie4uinit sólo existe en System32, al que se llega por Sysnative.
  ${If} ${FileExists} "$WINDIR\Sysnative\ie4uinit.exe"
    ExecWait '"$WINDIR\Sysnative\ie4uinit.exe" -show'
  ${ElseIf} ${FileExists} "$SYSDIR\ie4uinit.exe"
    ExecWait '"$SYSDIR\ie4uinit.exe" -show'
  ${EndIf}
!macroend
