//! 调试工具:枚举系统当前的 SMTC 会话(应用身份 + 媒体标题),用于验证
//! 控制中心里各会话的归属。运行:cargo run --example smtc-list

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
    use windows::Win32::System::Com::CoInitializeEx;
    unsafe {
        let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
    let sessions = manager.GetSessions()?;
    println!("SESSION_COUNT={}", sessions.Size()?);
    for s in &sessions {
        let app_id = s.SourceAppUserModelId()?;
        let props = s.TryGetMediaPropertiesAsync()?.get()?;
        let info = s.GetPlaybackInfo()?;
        let status = info.PlaybackStatus()?;
        let tl = s.GetTimelineProperties()?;
        let pos = tl.Position()?.Duration as f64 / 1e7;
        println!(
            "APP={} | TITLE={} | ARTIST={} | STATUS={:?} | POS={:.1}s",
            app_id,
            props.Title()?,
            props.Artist()?,
            status,
            pos
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("仅 Windows");
}
