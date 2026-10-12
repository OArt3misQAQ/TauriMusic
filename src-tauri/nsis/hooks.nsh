; TauriMusic 安装器钩子(NSIS)
; 自启动的开启/关闭由应用内设置管理(HKCU Run 键),安装过程不默认开启;
; 卸载时移除自启动条目,不留残余

!macro NSIS_HOOK_POSTUNINSTALL
  ; 应用内开启过自启动的话,卸载时一并移除
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "TauriMusic"
  ; 移除控制中心应用身份登记(SMTC 显示名/图标,含历史键名)
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\com.art3mis.taurimusic"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\tauri-music.exe"
!macroend
