// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// mimalloc 全局分配器:曲库/JSON 场景大量小分配,碎片率低于系统堆,常驻内存更省
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    // 进程 AUMID 显式设为 tauri identifier(必须早于任何窗口创建):
    // 系统媒体控制/控制中心按 AUMID 反查应用名,不设置时会话身份是裸 exe 名,
    // 查不到友好名显示「未知应用」;设成 identifier 后与应用目录里已有的
    // 「TauriMusic」记录对上号
    #[cfg(windows)]
    unsafe {
        let aumid = windows::core::HSTRING::from("com.art3mis.taurimusic");
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            windows::core::PCWSTR(aumid.as_ptr()),
        );
    }
    tauri_music_lib::run()
}
