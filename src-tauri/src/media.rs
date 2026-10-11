//! 媒体系统集成:Windows SMTC(系统媒体传输控制)+ 增强托盘菜单。
//! - SMTC:让 Windows 音量弹窗 / 锁屏 / 控制中心显示曲名、歌手、封面,
//!   并可直接播放/暂停/上一曲/下一曲(参照网易云 Windows 客户端的能力面)
//! - 托盘:右键显示正在播放(封面缩略图标 + 曲名艺人 + 实时进度文本)与播放控制
//!
//! 播放状态的唯一拥有者是前端 WebView(音频元素在那边),本模块只:
//! 接收前端推送的状态命令(曲目/状态/进度),把按钮事件 emit 回前端。
//! SMTC 是 COM 组件:全部操作收敛在一条专用线程(启动即 CoInitializeEx),
//! 命令经 channel 转发,避免各命令线程的 COM 套间问题。

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{IconMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

/// 托盘进度/曲名文本的上限:过长会把菜单撑得很宽
const TRAY_TEXT_MAX: usize = 48;

fn ellipsize(s: &str) -> String {
    if s.chars().count() <= TRAY_TEXT_MAX {
        s.to_string()
    } else {
        let cut: String = s.chars().take(TRAY_TEXT_MAX).collect();
        format!("{cut}…")
    }
}

/// 秒 → 「m:ss」(托盘进度文本)
pub(crate) fn fmt_clock(secs: f64) -> String {
    let s = if secs.is_finite() && secs > 0.0 {
        secs as u64
    } else {
        0
    };
    format!("{}:{:02}", s / 60, s % 60)
}

/// 托盘可变项的句柄缓存:避免重复 set_text 造成的菜单闪烁
pub(crate) struct TrayHandles {
    np: IconMenuItem<Wry>,
    prog: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    prog_cache: String,
    status_cache: Option<bool>,
}

/// 托盘 + SMTC 的共享桥
pub(crate) struct MediaBridge {
    pub tray: Mutex<Option<TrayHandles>>,
    #[cfg(windows)]
    pub(crate) smtc: Mutex<Option<std::sync::mpsc::Sender<SmtcMsg>>>,
}

/// 建托盘菜单并启动 SMTC;在 setup 里调用
pub(crate) fn init(app: &AppHandle) -> tauri::Result<()> {
    // 正在播放项:封面缩略图 + 曲名 - 艺人,点击唤起主窗口
    let np = IconMenuItem::with_id(app, "np", "未在播放", true, None::<Image>, None::<&str>)?;
    let prog = MenuItem::with_id(app, "prog", "0:00 / 0:00", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "暂停", true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", "上一曲", true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", "下一曲", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示 TauriMusic", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let tray_menu = Menu::with_items(
        app,
        &[
            &np, &prog, &sep1, &toggle, &prev, &next, &sep2, &show, &quit,
        ],
    )?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("缺少应用图标").clone())
        .tooltip("TauriMusic")
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            // 正在播放条目 / 显示:唤起主窗口
            "np" | "show" => crate::show_main(app),
            // 播放控制:按钮状态在前端,转发事件让前端驱动播放器
            "toggle" => {
                let _ = app.emit("media://action", "toggle");
            }
            "prev" => {
                let _ = app.emit("media://action", "prev");
            }
            "next" => {
                let _ = app.emit("media://action", "next");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::toggle_main(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(MediaBridge {
        tray: Mutex::new(Some(TrayHandles {
            np,
            prog,
            toggle,
            prog_cache: String::new(),
            status_cache: None,
        })),
        #[cfg(windows)]
        smtc: Mutex::new(None),
    });

    #[cfg(windows)]
    {
        let tx = spawn_smtc(app.clone());
        if let Some(b) = app.try_state::<MediaBridge>() {
            if let Ok(mut g) = b.smtc.lock() {
                *g = Some(tx);
            }
        }
    }
    Ok(())
}

// ===== 前端命令入口(lib.rs 的三个 #[tauri::command] 调到这里) =====

/// 曲目切换:更新托盘与 SMTC 元数据(含封面缩略图)
pub(crate) fn on_track(
    app: &AppHandle,
    title: String,
    artist: String,
    cover: Option<String>,
    duration: f64,
) {
    #[cfg(windows)]
    if let Some(b) = app.try_state::<MediaBridge>() {
        if let Ok(g) = b.smtc.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(SmtcMsg::Track {
                    title: title.clone(),
                    artist: artist.clone(),
                    cover: cover.clone().map(PathBuf::from),
                    duration,
                });
            }
        }
    }
    let _ = app.run_on_main_thread({
        let app = app.clone();
        move || tray_now_playing(&app, &title, &artist, cover.as_deref())
    });
}

/// 播放/暂停状态
pub(crate) fn on_status(app: &AppHandle, playing: bool) {
    #[cfg(windows)]
    if let Some(b) = app.try_state::<MediaBridge>() {
        if let Ok(g) = b.smtc.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(SmtcMsg::Status(playing));
            }
        }
    }
    let _ = app.run_on_main_thread({
        let app = app.clone();
        move || tray_status(&app, playing)
    });
}

/// 进度(节流后约 1 次/秒)
pub(crate) fn on_position(app: &AppHandle, position: f64, duration: f64) {
    #[cfg(windows)]
    if let Some(b) = app.try_state::<MediaBridge>() {
        if let Ok(g) = b.smtc.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(SmtcMsg::Position { position, duration });
            }
        }
    }
    let _ = app.run_on_main_thread({
        let app = app.clone();
        move || tray_progress(&app, position, duration)
    });
}

// ===== 托盘更新(必须在主线程:菜单是 UI 对象) =====

fn tray_now_playing(app: &AppHandle, title: &str, artist: &str, cover: Option<&str>) {
    let Some(b) = app.try_state::<MediaBridge>() else {
        return;
    };
    let Ok(mut g) = b.tray.lock() else { return };
    let Some(h) = g.as_mut() else { return };
    let text = if artist.is_empty() {
        ellipsize(title)
    } else {
        ellipsize(&format!("{title} - {artist}"))
    };
    let _ = h.np.set_text(&text);
    // 封面缩略图:本地缓存路径解码 → 32×32 RGBA;无封面时清掉图标
    let icon = cover.and_then(decode_icon);
    if let Err(e) = h.np.set_icon(icon) {
        eprintln!("[tray] 封面图标更新失败: {e}");
    }
}

fn tray_progress(app: &AppHandle, position: f64, duration: f64) {
    let Some(b) = app.try_state::<MediaBridge>() else {
        return;
    };
    let Ok(mut g) = b.tray.lock() else { return };
    let Some(h) = g.as_mut() else { return };
    let text = format!("{} / {}", fmt_clock(position), fmt_clock(duration));
    if h.prog_cache != text {
        h.prog_cache = text.clone();
        let _ = h.prog.set_text(&text);
    }
}

fn tray_status(app: &AppHandle, playing: bool) {
    let Some(b) = app.try_state::<MediaBridge>() else {
        return;
    };
    let Ok(mut g) = b.tray.lock() else { return };
    let Some(h) = g.as_mut() else { return };
    if h.status_cache != Some(playing) {
        h.status_cache = Some(playing);
        let _ = h.toggle.set_text(if playing { "暂停" } else { "播放" });
    }
}

/// 封面文件 → 32×32 RGBA(tauri Image)
fn decode_icon(path: &str) -> Option<Image<'static>> {
    let img = image::open(path).ok()?;
    let thumb = img
        .resize_to_fill(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (w, h) = (thumb.width(), thumb.height());
    Some(Image::new_owned(thumb.into_raw(), w, h))
}

// ===== Windows SMTC =====

#[cfg(windows)]
#[derive(Debug)]
pub(crate) enum SmtcMsg {
    Track {
        title: String,
        artist: String,
        cover: Option<PathBuf>,
        duration: f64,
    },
    Status(bool),
    Position {
        position: f64,
        duration: f64,
    },
}

/// 启动 SMTC 专用线程(全部 WinRT 操作都在这条已初始化 COM 的线程上)
#[cfg(windows)]
fn spawn_smtc(app: AppHandle) -> std::sync::mpsc::Sender<SmtcMsg> {
    let (tx, rx) = std::sync::mpsc::channel::<SmtcMsg>();
    std::thread::Builder::new()
        .name("smtc".into())
        .spawn(move || {
            if let Err(e) = smtc_loop(&app, rx) {
                // SMTC 初始化失败只降级(无系统媒体控制),不影响应用本体
                eprintln!("[smtc] 初始化失败: {e}");
            }
        })
        .expect("创建 smtc 线程失败");
    tx
}

#[cfg(windows)]
fn smtc_loop(app: &AppHandle, rx: std::sync::mpsc::Receiver<SmtcMsg>) -> windows::core::Result<()> {
    use windows::core::{factory, HSTRING};
    use windows::Foundation::TimeSpan;
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::RandomAccessStreamReference;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::CoInitializeEx;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

    // WebView2 已在主线程初始化 COM;专用线程自己初始化 MTA。
    // 若模式不匹配(极少)忽略错误——WinRT 调用仍可用
    unsafe {
        let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }

    let hwnd = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .ok_or_else(|| windows::core::Error::from(windows::core::HRESULT(-1)))?;
    // tauri 依赖树与本 crate 的 windows 版本一致时无需转换;字段类型统一为 *mut c_void
    let hwnd = HWND(hwnd.0);

    let interop: ISystemMediaTransportControlsInterop =
        factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
    let _ = controls.SetIsEnabled(true);
    let _ = controls.SetIsPlayEnabled(true);
    let _ = controls.SetIsPauseEnabled(true);
    let _ = controls.SetIsNextEnabled(true);
    let _ = controls.SetIsPreviousEnabled(true);
    let _ = controls.SetPlaybackStatus(MediaPlaybackStatus::Stopped);

    // 注册系统媒体按钮的 WinRT 事件回调:纯进程内事件订阅,与任何 shell 无关。
    // 0.61 的 ButtonPressed 收泛型 TypedEventHandler,提供闭包即可
    let on_button = app.clone();
    let button_cb = windows::Foundation::TypedEventHandler::<
        SystemMediaTransportControls,
        SystemMediaTransportControlsButtonPressedEventArgs,
    >::new(move |_sender, args| {
        let args = args.ok()?;
        let button = args.Button()?;
        // 伪枚举是 struct + 常量,不能用 match
        let action = if button == SystemMediaTransportControlsButton::Play {
            "play"
        } else if button == SystemMediaTransportControlsButton::Pause {
            "pause"
        } else if button == SystemMediaTransportControlsButton::Next {
            "next"
        } else if button == SystemMediaTransportControlsButton::Previous {
            "prev"
        } else if button == SystemMediaTransportControlsButton::Stop {
            "stop"
        } else {
            ""
        };
        if !action.is_empty() {
            let _ = on_button.emit("media://action", action);
        }
        Ok(())
    });
    let button_token = controls.ButtonPressed(&button_cb)?;
    let _ = button_token;

    let du = controls.DisplayUpdater()?;
    let _ = du.SetType(MediaPlaybackType::Music);
    let _ = du.Update();

    // 常驻时间轴缓存:进度更新时保留 EndTime(先在 Track 分支赋值)
    #[allow(unused_assignments)]
    let mut end_time: i64 = 0;
    for msg in rx {
        let apply = |r: windows::core::Result<()>| {
            if let Err(e) = r {
                eprintln!("[smtc] 更新失败: {e}");
            }
        };
        match msg {
            SmtcMsg::Track {
                title,
                artist,
                cover,
                duration,
            } => {
                end_time = (duration * 1e7) as i64;
                apply((|| {
                    // MusicDisplayProperties 不可直接构造,由 DisplayUpdater 属性取得
                    let mp = du.MusicProperties()?;
                    mp.SetTitle(&HSTRING::from(&title))?;
                    mp.SetArtist(&HSTRING::from(&artist))?;
                    if let Some(p) = cover {
                        // 0.61 的 CreateFromFile 收 IStorageFile:按路径取 StorageFile(异步转阻塞)
                        let file = windows::Storage::StorageFile::GetFileFromPathAsync(
                            &HSTRING::from(p.as_os_str().to_string_lossy().as_ref()),
                        )?
                        .get()?;
                        du.SetThumbnail(&RandomAccessStreamReference::CreateFromFile(&file)?)?;
                    }
                    du.Update()?;
                    Ok(())
                })());
                // TimelineProperties 是激活型类:new + setter,而非字段赋值
                apply((|| {
                    let tl = SystemMediaTransportControlsTimelineProperties::new()?;
                    tl.SetPosition(TimeSpan { Duration: 0 })?;
                    tl.SetEndTime(TimeSpan { Duration: end_time })?;
                    tl.SetMinSeekTime(TimeSpan { Duration: 0 })?;
                    tl.SetMaxSeekTime(TimeSpan { Duration: end_time })?;
                    controls.UpdateTimelineProperties(&tl)?;
                    Ok(())
                })());
            }
            SmtcMsg::Status(playing) => {
                apply(controls.SetPlaybackStatus(if playing {
                    MediaPlaybackStatus::Playing
                } else {
                    MediaPlaybackStatus::Paused
                }));
            }
            SmtcMsg::Position { position, duration } => {
                end_time = (duration * 1e7) as i64;
                apply((|| {
                    let tl = SystemMediaTransportControlsTimelineProperties::new()?;
                    tl.SetPosition(TimeSpan {
                        Duration: (position * 1e7) as i64,
                    })?;
                    tl.SetEndTime(TimeSpan { Duration: end_time })?;
                    tl.SetMinSeekTime(TimeSpan { Duration: 0 })?;
                    tl.SetMaxSeekTime(TimeSpan { Duration: end_time })?;
                    controls.UpdateTimelineProperties(&tl)?;
                    Ok(())
                })());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn fmt_clock_renders_m_ss() {
        use super::fmt_clock;
        // 常规值与边界:负数/NaN 一律按 0 处理
        assert_eq!(fmt_clock(0.0), "0:00");
        assert_eq!(fmt_clock(61.4), "1:01");
        assert_eq!(fmt_clock(3599.9), "59:59");
        assert_eq!(fmt_clock(-3.0), "0:00");
        assert_eq!(fmt_clock(f64::NAN), "0:00");
    }

    #[test]
    fn ellipsize_truncates_long_text() {
        use super::ellipsize;
        assert_eq!(ellipsize("短标题 - 艺人"), "短标题 - 艺人");
        let long = "曲".repeat(60);
        let out = ellipsize(&long);
        // 截到上限并带省略号
        assert!(out.chars().count() <= super::TRAY_TEXT_MAX + 1);
        assert!(out.ends_with('…'));
    }
}
