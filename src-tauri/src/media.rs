//! 媒体系统集成:Windows SMTC(系统媒体传输控制)+ 托盘弹窗。
//! - SMTC:让 Windows 音量弹窗 / 锁屏 / 控制中心显示曲名、歌手、封面,
//!   并可直接播放/暂停/上一曲/下一曲(参照网易云 Windows 客户端的能力面)
//! - 托盘:左键唤起主窗口;右键弹出原生菜单装不下的迷你播放面板
//!   (封面 + 曲名 + 真实进度条 + 播放控制,由前端 TrayPopup 组件渲染)
//!
//! 播放状态的唯一拥有者是前端 WebView(音频元素在那边),本模块只:
//! 接收前端推送的状态命令(曲目/状态/进度),把按钮事件 emit 回前端。
//! SMTC 是 COM 组件:全部操作收敛在一条专用线程(启动即 CoInitializeEx),
//! 命令经 channel 转发,避免各命令线程的 COM 套间问题。

// 以下三组导入均只在 Windows(SMTC)代码路径使用,非 Windows 编译时不存在
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::Mutex;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
#[cfg(windows)]
use tauri::Emitter;
use tauri::{AppHandle, Manager};

// 托盘弹窗的逻辑尺寸(物理像素 = 逻辑 × 显示缩放,定位时换算)
const POPUP_W: f64 = 320.0;
const POPUP_H: f64 = 152.0;

/// 托盘 + SMTC 的共享桥
pub(crate) struct MediaBridge {
    #[cfg(windows)]
    pub(crate) smtc: Mutex<Option<std::sync::mpsc::Sender<SmtcMsg>>>,
}

/// 建托盘图标并启动 SMTC;在 setup 里调用
pub(crate) fn init(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("缺少应用图标").clone())
        .tooltip("TauriMusic")
        // 右键弹出迷你面板而非原生菜单:原生 Win32 菜单只有纯文本行,
        // 放不下进度条与封面,质感上限太低
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                match button {
                    // 左键:切换主窗口显示/隐藏
                    MouseButton::Left => crate::toggle_main(tray.app_handle()),
                    // 右键:在图标上方弹出迷你播放面板(坐标统一换算成物理像素)
                    MouseButton::Right => {
                        let (cx, top, ih) = match (rect.position, rect.size) {
                            (tauri::Position::Physical(p), tauri::Size::Physical(s)) => (
                                p.x as f64 + s.width as f64 / 2.0,
                                p.y as f64,
                                s.height as f64,
                            ),
                            (tauri::Position::Logical(p), tauri::Size::Logical(s)) => {
                                (p.x + s.width / 2.0, p.y, s.height)
                            }
                            _ => (0.0, 0.0, 0.0),
                        };
                        show_tray_popup(tray.app_handle(), cx, top, ih);
                    }
                    _ => {}
                }
            }
        })
        .build(app)?;

    app.manage(MediaBridge {
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

/// 把迷你面板摆到托盘图标正上方:图标中点水平居中,底边距图标顶 8px;
/// 越界时按所在显示器夹取,任务栏在顶部时改到图标下方。
/// 图标几何量(中点 x/顶边 y/高度)为物理像素
fn show_tray_popup(app: &AppHandle, icon_cx: f64, icon_top: f64, icon_h: f64) {
    let Some(w) = app.get_webview_window("tray") else {
        return;
    };
    let Ok(Some(monitor)) = app.monitor_from_point(icon_cx, icon_top) else {
        return;
    };
    let scale = monitor.scale_factor();
    let (pw, ph) = (POPUP_W * scale, POPUP_H * scale);

    let mx = monitor.position().x as f64;
    let my = monitor.position().y as f64;
    let mw = monitor.size().width as f64;

    let x = (icon_cx - pw / 2.0).clamp(mx + 4.0, mx + mw - pw - 4.0);
    let mut y = icon_top - ph - 8.0;
    if y < my + 4.0 {
        // 任务栏在屏幕顶部:面板改到图标下方
        y = icon_top + icon_h + 8.0;
    }
    let _ = w.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    let _ = w.show();
    // 接管焦点:失焦即隐藏(见 lib.rs 的窗口事件)
    let _ = w.set_focus();
}

// ===== 前端命令入口(lib.rs 的三个 #[tauri::command] 调到这里) =====

/// 曲目切换:推送 SMTC 元数据(含封面)
pub(crate) fn on_track(
    app: &AppHandle,
    title: String,
    artist: String,
    cover: Option<String>,
    duration: f64,
) {
    // 非 Windows:SMTC 通道不存在,全部参数仅 Windows 路径使用
    #[cfg(not(windows))]
    let _ = (app, title, artist, cover, duration);
    #[cfg(windows)]
    if let Some(b) = app.try_state::<MediaBridge>() {
        if let Ok(g) = b.smtc.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(SmtcMsg::Track {
                    title,
                    artist,
                    cover: cover.map(PathBuf::from),
                    duration,
                });
            }
        }
    }
}

/// 播放/暂停状态
pub(crate) fn on_status(app: &AppHandle, playing: bool) {
    #[cfg(not(windows))]
    let _ = (app, playing);
    #[cfg(windows)]
    if let Some(b) = app.try_state::<MediaBridge>() {
        if let Ok(g) = b.smtc.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(SmtcMsg::Status(playing));
            }
        }
    }
}

/// 进度(节流后约 1 次/秒)
pub(crate) fn on_position(app: &AppHandle, position: f64, duration: f64) {
    #[cfg(not(windows))]
    let _ = (app, position, duration);
    #[cfg(windows)]
    if let Some(b) = app.try_state::<MediaBridge>() {
        if let Ok(g) = b.smtc.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(SmtcMsg::Position { position, duration });
            }
        }
    }
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
        .spawn(move || run_smtc(app, rx))
        .expect("创建 smtc 线程失败");
    tx
}

/// 线程体:SMTC 初始化失败只降级(无系统媒体控制),不影响应用本体
#[cfg(windows)]
fn run_smtc(app: AppHandle, rx: std::sync::mpsc::Receiver<SmtcMsg>) {
    if let Err(e) = smtc_loop(&app, rx) {
        eprintln!("[smtc] 初始化失败: {e}");
    }
}

// 系统媒体按钮回调的委托类型(泛型事件接口的具体化)
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

    // 系统媒体按钮回调的委托类型(泛型事件接口的具体化)
    type ButtonEventCb = windows::Foundation::TypedEventHandler<
        SystemMediaTransportControls,
        SystemMediaTransportControlsButtonPressedEventArgs,
    >;

    // WebView2 已在主线程初始化 COM;专用线程自己初始化 MTA。
    // 若模式不匹配(极少)忽略错误——WinRT 调用仍可用
    unsafe {
        let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }

    let hwnd = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .ok_or_else(|| windows::core::Error::from(windows::core::HRESULT(-1)))?;
    // tauri 依赖树与本 crate 的 windows 版本一致,字段类型统一为 *mut c_void
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

    // 注册系统媒体按钮的 WinRT 事件回调:纯进程内事件订阅,与任何 shell 无关
    let on_button = app.clone();
    let button_cb = ButtonEventCb::new(move |_sender, args| {
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
