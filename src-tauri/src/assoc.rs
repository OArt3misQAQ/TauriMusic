//! 文件关联与开机自启动:全部写 HKCU 注册表,用户级无需管理员。

// ===== 格式关联(HKCU 注册到"打开方式",无需管理员) =====

pub(crate) const ASSOC_EXTS: &[(&str, &str)] = &[
    ("tmc", "TMC 音乐包"),
    ("tmcl", "TMCL 播放列表"),
    ("tmca", "TMCA 专辑包"),
    ("mp3", "MP3 音频"),
    ("flac", "FLAC 音频"),
    ("m4a", "M4A 音频"),
    ("wav", "WAV 音频"),
    ("ogg", "OGG 音频"),
    ("opus", "OPUS 音频"),
];

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssocState {
    ext: String,
    label: String,
    registered: bool,
}

fn assoc_desc(ext: &str) -> &'static str {
    ASSOC_EXTS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, d)| *d)
        .unwrap_or("音频")
}

pub(crate) fn get_associations() -> Vec<AssocState> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        ASSOC_EXTS
            .iter()
            .map(|(ext, label)| {
                let registered = hkcu
                    .open_subkey(format!(r"Software\Classes\.{ext}\OpenWithProgids"))
                    .ok()
                    .map(|k| {
                        k.get_value::<String, _>(format!("TauriMusic.{ext}"))
                            .is_ok()
                    })
                    .unwrap_or(false);
                AssocState {
                    ext: ext.to_string(),
                    label: label.to_string(),
                    registered,
                }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        ASSOC_EXTS
            .iter()
            .map(|(ext, label)| AssocState {
                ext: ext.to_string(),
                label: label.to_string(),
                registered: false,
            })
            .collect()
    }
}

pub(crate) fn set_association(ext: String, enable: bool) -> Result<(), String> {
    // ext 来自前端 invoke:必须限定在支持名单内。get_associations 只读白名单,
    // 这里若不设防,被攻破的前端可借本命令改写任意 HKCU 类注册表
    // (如 .exe 的用户级默认 ProgID,或拼出带 \ 的子键路径)
    if !ASSOC_EXTS.iter().any(|(e, _)| *e == ext) {
        return Err(format!("不支持的扩展名: .{ext}"));
    }
    let desc = assoc_desc(&ext);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let progid = format!("TauriMusic.{ext}");
        if enable {
            let cmd_key = hkcu
                .create_subkey(format!(r"Software\Classes\{progid}\shell\open\command"))
                .map_err(|e| e.to_string())?
                .0;
            cmd_key
                .set_value("", &format!("\"{}\" \"%1\"", exe.display()))
                .map_err(|e| e.to_string())?;
            let icon_key = hkcu
                .create_subkey(format!(r"Software\Classes\{progid}\DefaultIcon"))
                .map_err(|e| e.to_string())?
                .0;
            icon_key
                .set_value("", &format!("{},0", exe.display()))
                .map_err(|e| e.to_string())?;
            let id_key = hkcu
                .create_subkey(format!(r"Software\Classes\{progid}"))
                .map_err(|e| e.to_string())?
                .0;
            id_key
                .set_value("", &format!("TauriMusic {desc}"))
                .map_err(|e| e.to_string())?;
            let ext_key = hkcu
                .create_subkey(format!(r"Software\Classes\.{ext}\OpenWithProgids"))
                .map_err(|e| e.to_string())?
                .0;
            ext_key
                .set_value(&progid, &String::new())
                .map_err(|e| e.to_string())?;
            // 用户级默认 ProgID:无 UserChoice 时双击即用 TauriMusic(优先于 HKLM)
            let def_key = hkcu
                .create_subkey(format!(r"Software\Classes\.{ext}"))
                .map_err(|e| e.to_string())?
                .0;
            def_key.set_value("", &progid).map_err(|e| e.to_string())?;
        } else {
            if let Ok(k) = hkcu.open_subkey_with_flags(
                format!(r"Software\Classes\.{ext}\OpenWithProgids"),
                KEY_WRITE,
            ) {
                let _ = k.delete_value(&progid);
            }
            // 仅当用户级默认仍指向我们时才摘除,不破坏其他应用的关联
            if let Ok(k) =
                hkcu.open_subkey_with_flags(format!(r"Software\Classes\.{ext}"), KEY_WRITE)
            {
                let current: Result<String, _> = k.get_value("");
                if matches!(current, Ok(v) if v == progid) {
                    let _ = k.delete_value("");
                }
            }
            let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{progid}"));
        }
        // 通知 Explorer 立即刷新关联与图标缓存
        #[link(name = "shell32")]
        extern "system" {
            fn SHChangeNotify(
                wEventId: u32,
                uFlags: u32,
                dwItem1: *const std::ffi::c_void,
                dwItem2: *const std::ffi::c_void,
            );
        }
        unsafe {
            SHChangeNotify(0x0800_0000, 0x0000, std::ptr::null(), std::ptr::null());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (desc, exe, enable);
        Err("仅支持 Windows".into())
    }
}

// ===== 自启动(HKCU Run 键,用户级,无需管理员;与文件关联同一套注册表写法) =====

// 仅 Windows 代码路径引用:不加 cfg 的话 Linux 编译时是死代码,clippy -D warnings 会判死
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "TauriMusic";

/// 查询是否已开启开机自启动(HKCU Run 键下有无 TauriMusic 条目)
pub(crate) fn get_autostart() -> bool {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(RUN_KEY)
            .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
            .is_ok()
    }
    #[cfg(not(windows))]
    false
}

/// 开启/关闭开机自启动:写或删 HKCU Run 键的 TauriMusic 条目(值为当前 exe 路径)
pub(crate) fn set_autostart(enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let run = hkcu
            .open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        if enable {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            run.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()))
                .map_err(|e| e.to_string())?;
        } else {
            run.delete_value(RUN_VALUE).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = enable;
        Err("仅支持 Windows".into())
    }
}
