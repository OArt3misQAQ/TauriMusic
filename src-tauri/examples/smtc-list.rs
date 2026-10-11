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
        println!(
            "APP={} | TITLE={} | ARTIST={}",
            app_id,
            props.Title()?,
            props.Artist()?
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("仅 Windows");
}
