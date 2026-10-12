//! 启动守卫：已有实例在运行时，询问用户是否结束旧进程并重启。
//!
//! 为什么要在 Builder 之前做这件事：tauri_plugin_single_instance 的"唤起已有窗口"
//! 依赖旧实例的事件循环活着；一旦旧实例卡死，转发石沉大海，用户会以为没打开而连点，
//! 结果是几十个新进程排队闪退、界面毫无反馈。本模块用"新实例自己检测并询问"兜住卡死场景：
//!
//! 1. sysinfo 枚举同名进程（排除自身）；
//! 2. 最老的旧实例仍在启动宽限期内 → 视为连点/冷启动竞态，不弹窗，放行给插件处理；
//! 3. "询问"本身用命名互斥体去重 —— 连点风暴下只有第一个进程弹窗，其余静默放行，绝不叠窗；
//! 4. 选重启 → 结束全部旧进程后由本进程接管启动；选否 → 放行给插件（旧窗口被唤起，本次启动退出）。
//!
//! 最终"谁是主实例"仍由 single-instance 插件的互斥体裁定，本模块只负责询问与清场，
//! 两层职责互不越界。非 Windows 平台仅为 CI 编译保留（应用只发 Windows 包）。

use std::ffi::OsStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 宽限期（秒）：此内启动的同名进程视为"正在启动的兄弟实例"，不询问
pub(crate) const GRACE_SECS: u64 = 4;
/// 选重启后等待旧进程退出的上限：超时说明有进程结束不掉（权限不足等），只能放弃
const KILL_WAIT: Duration = Duration::from_millis(5000);
/// 询问去重互斥体名：Local\ 前缀 = 每登录会话独立，多用户互不干扰
#[cfg(windows)]
const ASK_MUTEX_NAME: &str = "Local\\TauriMusic-StartupAsk";

/// 启动决策（纯逻辑，两端可单测）
pub(crate) enum StartupPlan {
    /// 放行给 single-instance 插件：正常首启 / 兄弟实例宽限期内 / 已有兄弟在询问
    DeferToPlugin,
    /// 向用户询问是否重启
    AskRestart,
}

/// true = 继续本次启动；false = 放弃本次启动（进程直接退出）
pub fn guard_startup() -> bool {
    // 文件关联启动（双击 .tmc/音频文件）必须走插件转发链路把文件交给运行中的实例，
    // 若在这里弹"是否重启"会打断"双击音乐包 → 导入"的直觉流程
    if has_open_file_arg(&std::env::args().collect::<Vec<_>>()) {
        return true;
    }
    let others = find_others();
    let oldest = others.iter().map(|o| o.age_secs).max();
    match plan_startup(!others.is_empty(), oldest) {
        StartupPlan::DeferToPlugin => true,
        StartupPlan::AskRestart => ask_restart_or_defer(),
    }
}

/// 询问重启或放行。返回 true 表示继续本次启动（重启接管或交给插件裁定）。
fn ask_restart_or_defer() -> bool {
    // 关键防御：拿不到询问锁说明已有兄弟实例正在弹窗 —— 宁可少问，绝不叠第二个窗
    let _ask = match ask_lock::acquire() {
        Some(lock) => lock,
        None => return true,
    };
    if !ask_restart_dialog() {
        // 用户选否：放行给插件，旧窗口被唤起，本次启动随后自动退出
        return true;
    }
    if kill_others_and_wait() {
        // 旧进程已清场，继续启动即可在插件处成为新主实例
        true
    } else {
        error_dialog();
        false
    }
}

/// 有同名进程在运行时，结合"最老实例的年龄"决定询问还是放行
fn plan_startup(others_present: bool, oldest_age_secs: Option<u64>) -> StartupPlan {
    if !others_present {
        return StartupPlan::DeferToPlugin;
    }
    match oldest_age_secs {
        // 最老的兄弟都还在宽限期内：这是连点/冷启动竞态，交给插件聚焦已有窗口即可
        Some(age) if age < GRACE_SECS => StartupPlan::DeferToPlugin,
        // 读不到启动时间（进程信息缺失）按保守处理：宁可询问，也不能让用户以为应用打不开
        _ => StartupPlan::AskRestart,
    }
}

/// argv 中是否带有真实存在的文件路径（文件关联双击启动的特征；args[0] 是 exe 本身要跳过）
pub(crate) fn has_open_file_arg(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|a| std::path::Path::new(a).is_file())
}

/// 同名进程判定：比较进程名与自身可执行文件名，大小写不敏感
/// （Windows 文件系统大小写不敏感，任务管理器里 "TauriMusic.exe" 与 "taurimusic.exe" 是同一个）
pub(crate) fn is_same_exe(process_name: &OsStr, self_name: &OsStr) -> bool {
    process_name.to_string_lossy().to_lowercase() == self_name.to_string_lossy().to_lowercase()
}

struct OtherInstance {
    pid: sysinfo::Pid,
    age_secs: u64,
}

/// 枚举除自身以外的同名进程。拿不到自身路径时宁可放行（误杀他人代价远大于漏检）。
fn find_others() -> Vec<OtherInstance> {
    let Some(self_name) = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
    else {
        return Vec::new();
    };
    let self_pid = sysinfo::get_current_pid().ok();
    let sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::default().with_processes(sysinfo::ProcessRefreshKind::default()),
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    sys.processes()
        .iter()
        .filter(|(pid, p)| Some(**pid) != self_pid && is_same_exe(p.name(), &self_name))
        .map(|(pid, p)| OtherInstance {
            pid: *pid,
            age_secs: now.saturating_sub(p.start_time()),
        })
        .collect()
}

/// 结束所有同名旧进程并等待其真正退出。
/// 每轮都重新枚举补刀：对话框打开期间新冒出来的兄弟实例也要一并清掉，
/// 否则它会在旧进程死后抢到插件互斥体，变成"看不见的幽灵实例"。
fn kill_others_and_wait() -> bool {
    let deadline = Instant::now() + KILL_WAIT;
    loop {
        let targets = find_others();
        if targets.is_empty() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        let sys = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::default().with_processes(sysinfo::ProcessRefreshKind::default()),
        );
        for t in &targets {
            if let Some(p) = sys.process(t.pid) {
                // 强杀而非 WM_CLOSE：应用点 × 只是隐藏到托盘，WM_CLOSE 根本退不出进程
                p.kill();
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn ask_restart_dialog() -> bool {
    rfd::MessageDialog::new()
        .set_title("TauriMusic 已在运行")
        .set_level(rfd::MessageLevel::Info)
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_description("检测到 TauriMusic 正在运行。\n\n「是」结束现有进程并重新启动；「否」唤起已运行的窗口。")
        .show()
        == rfd::MessageDialogResult::Yes
}

fn error_dialog() {
    rfd::MessageDialog::new()
        .set_title("TauriMusic")
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .set_description("未能结束已运行的 TauriMusic 进程，请手动关闭后重新启动。")
        .show();
}

#[cfg(windows)]
mod ask_lock {
    use super::ASK_MUTEX_NAME;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE,
    };
    use windows::Win32::System::Threading::CreateMutexW;

    /// 询问锁：持有期间本实例是唯一弹窗者。
    /// 用命名互斥体而不是锁文件，是因为互斥体随进程生死自动释放——
    /// 弹窗进程崩溃也不会把后续启动永久锁死。
    pub(super) struct AskLock {
        _handle: HANDLE,
    }

    pub(super) fn acquire() -> Option<AskLock> {
        let mut name: Vec<u16> = ASK_MUTEX_NAME.encode_utf16().collect();
        name.push(0);
        unsafe {
            // CreateMutexW 在互斥体已存在时仍返回有效句柄，只能靠 GetLastError 区分；
            // windows crate 仅在返回失败时读取 LastError，所以先手动清零再比较
            SetLastError(ERROR_SUCCESS);
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                None
            } else {
                Some(AskLock { _handle: handle })
            }
        }
    }

    impl Drop for AskLock {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self._handle);
            }
        }
    }
}

#[cfg(not(windows))]
mod ask_lock {
    /// 非 Windows 仅为 CI 编译保留：不做去重（应用只发 Windows 包，且决策纯逻辑已在 tests 覆盖）
    pub(super) struct AskLock;

    pub(super) fn acquire() -> Option<AskLock> {
        Some(AskLock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_respects_grace_period() {
        // 无同名实例：放行，正常首启
        assert!(matches!(
            plan_startup(false, None),
            StartupPlan::DeferToPlugin
        ));
        // 刚启动 1 秒的兄弟实例：视为连点竞态，不弹窗，交给插件聚焦已有窗口
        assert!(matches!(
            plan_startup(true, Some(1)),
            StartupPlan::DeferToPlugin
        ));
        // 恰好到宽限期边界：已算稳定运行，应当询问
        assert!(matches!(
            plan_startup(true, Some(GRACE_SECS)),
            StartupPlan::AskRestart
        ));
    }

    #[test]
    fn plan_asks_when_age_unknown() {
        // 有同名进程但读不到启动时间：保守询问，不能默默退出让用户以为应用打不开
        assert!(matches!(plan_startup(true, None), StartupPlan::AskRestart));
    }

    #[test]
    fn exe_name_match_is_case_insensitive() {
        // Windows 文件系统大小写不敏感，同名判定必须忽略大小写
        assert!(is_same_exe(
            OsStr::new("TauriMusic.exe"),
            OsStr::new("taurimusic.EXE")
        ));
        // 不同进程名不能误伤
        assert!(!is_same_exe(
            OsStr::new("TauriMusic.exe"),
            OsStr::new("explorer.exe")
        ));
    }

    #[test]
    fn file_arg_detected_only_when_exists() {
        // 文件关联启动（双击音乐包/音频）必须走转发链路，不询问重启
        let tmp = std::env::temp_dir().join(format!("tm-guard-test-{}.tmp", std::process::id()));
        std::fs::write(&tmp, b"x").unwrap();
        assert!(has_open_file_arg(&[
            "TauriMusic.exe".to_string(),
            tmp.to_string_lossy().to_string()
        ]));
        // 不存在的路径不算文件参数（可能是启动器传的开关之类）
        assert!(!has_open_file_arg(&[
            "TauriMusic.exe".to_string(),
            "Z:\\不存在.tmc".to_string()
        ]));
        std::fs::remove_file(&tmp).ok();
    }
}
