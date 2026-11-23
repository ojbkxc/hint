//! `hint --doctor`：不启动界面的自助诊断。
//!
//! 用户在反馈「打不开 / 没声音 / 又要下模型」时，最需要的是把环境事实一次性
//! 收集齐：版本、数据目录、模型就绪态、配置与 Key、数据库版本、音频设备、
//! 最近日志错误。全部结果同时打印到控制台（若能附上父控制台）并写入
//! `<app_data_dir>/doctor-report.txt`——release 构建是 windows 子系统、
//! 默认没有控制台，文件是唯一可靠出口。
//!
//! 只读：不下载、不改配置、不创建会话。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// 诊断报告落盘文件名。
const REPORT_NAME: &str = "doctor-report.txt";

/// 读取最近日志错误的最大行数（从尾部回看）。
const LOG_TAIL_LINES: usize = 200;

/// 运行诊断并把报告写到 `app_data_dir`；返回进程退出码（0 正常，1 有阻塞项）。
pub(crate) fn run(app_data_dir: &Path) -> i32 {
    let report = collect(app_data_dir);
    let _ = attach_parent_console();
    print!("{report}");
    let path = app_data_dir.join(REPORT_NAME);
    let saved = std::fs::write(&path, &report).is_ok();
    if saved {
        println!("\n报告已保存：{}", path.display());
    } else {
        eprintln!("\n报告写入失败（目录不可写）：{}", path.display());
    }
    if report.contains("[阻塞]") { 1 } else { 0 }
}

/// 收集全部诊断信息，返回可直接打印的文本。
fn collect(app_data_dir: &Path) -> String {
    let mut out = String::new();
    let mut blocking = 0usize;

    let _ = writeln!(out, "hint 诊断报告");
    let _ = writeln!(out, "版本：{}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        out,
        "时间：{}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    let _ = writeln!(out, "数据目录：{}", app_data_dir.display());
    let _ = writeln!(out);

    // —— 语音模型 ——
    // 模型根与实际安装位置：1.0.12 起放在 %LOCALAPPDATA%\hint-models，
    // 旧版在数据目录下的 voice/。两个都探，便于判断「升级后是否需要重下」。
    let model_root = model_root_for_report(app_data_dir);
    let manager = hint_stt::ModelManager::new(model_root.clone());
    let _ = writeln!(out, "【语音模型】");
    let _ = writeln!(out, "  模型根目录：{}", manager.model_dir().display());
    match manager.status() {
        hint_stt::ModelStatus::Ready { model_dir } => {
            let _ = writeln!(out, "  状态：已就绪");
            let _ = writeln!(out, "  安装位置：{model_dir}");
        }
        hint_stt::ModelStatus::Downloading {
            step,
            resource,
            completed_bytes,
            total_bytes,
        } => {
            let _ = writeln!(
                out,
                "  状态：下载中（{} {}，{} / {}）",
                step.label(),
                resource,
                hint_core::diagnostics::format_mb(completed_bytes),
                hint_core::diagnostics::format_mb(total_bytes)
            );
        }
        hint_stt::ModelStatus::Unprepared {
            downloaded_bytes,
            total_bytes,
        } => {
            let _ = writeln!(
                out,
                "  状态：未就绪（已下载 {} / 共 {}）",
                hint_core::diagnostics::format_mb(downloaded_bytes),
                hint_core::diagnostics::format_mb(total_bytes)
            );
            if downloaded_bytes == 0 {
                let _ = writeln!(out, "  [阻塞] 语音模型尚未下载，无法开始面试");
                blocking += 1;
            } else {
                let _ = writeln!(out, "  提示：有断点续传数据，可在设置页继续下载");
            }
        }
    }
    // 旧位置残留（升级后未迁移或迁移失败）单独提示。
    let legacy = app_data_dir.join("voice");
    if legacy.is_dir() && legacy != model_root {
        let _ = writeln!(
            out,
            "  提示：旧模型目录仍存在（{}），若非本次升级迁移残留可手动删除",
            legacy.display()
        );
    }
    let _ = writeln!(out);

    // —— 配置与凭据 ——
    let settings_path = app_data_dir.join("settings.json");
    let settings = hint_core::settings::Settings::load(&settings_path);
    let _ = writeln!(out, "【配置】");
    let _ = writeln!(
        out,
        "  配置文件：{}{}",
        settings_path.display(),
        if settings_path.exists() {
            ""
        } else {
            "（不存在，使用默认值）"
        }
    );
    let provider = settings.provider.as_str();
    let _ = writeln!(out, "  协议：{provider}");
    let has_key = hint_core::secret::has(provider);
    let _ = writeln!(
        out,
        "  API Key：{}",
        if has_key {
            "已保存（Windows 凭据管理器）"
        } else {
            "未保存"
        }
    );
    if !has_key {
        let _ = writeln!(out, "  [阻塞] 当前协议没有 API Key，无法生成答案");
        blocking += 1;
    }
    // Base URL / 模型名只报「是否填写」，不打印内容——避免把用户的自建网关域名
    // 或账号信息写进随手转发的报告。
    let _ = writeln!(
        out,
        "  Base URL：{}",
        if settings.base_url.trim().is_empty() {
            "未填写"
        } else {
            "已填写"
        }
    );
    let _ = writeln!(
        out,
        "  模型名：{}",
        if settings.model.trim().is_empty() {
            "未填写"
        } else {
            "已填写"
        }
    );
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        let _ = writeln!(out, "  [阻塞] Base URL 或模型名为空");
        blocking += 1;
    }
    let _ = writeln!(
        out,
        "  浮层不透明度：{:.0}%",
        settings.overlay_opacity * 100.0
    );
    let _ = writeln!(
        out,
        "  麦克风：{}",
        hint_core::diagnostics::device_label(&settings.mic_device, "系统默认")
    );
    let _ = writeln!(
        out,
        "  系统输出：{}",
        hint_core::diagnostics::device_label(&settings.output_device, "系统默认")
    );
    let _ = writeln!(out);

    // —— 会话数据库 ——
    let db_path = app_data_dir.join("hint.db");
    let _ = writeln!(out, "【会话数据库】");
    let _ = writeln!(out, "  文件：{}", db_path.display());
    if db_path.exists() {
        match hint_store::Store::open(&db_path) {
            Ok(store) => {
                let version: i64 = store
                    .connection()
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .unwrap_or(-1);
                let integrity: String = store
                    .connection()
                    .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                    .unwrap_or_else(|error| format!("检查失败：{error}"));
                let _ = writeln!(out, "  结构版本：{version}");
                let _ = writeln!(out, "  完整性：{integrity}");
                if integrity != "ok" {
                    let _ = writeln!(out, "  [阻塞] 数据库完整性检查未通过");
                    blocking += 1;
                }
            }
            Err(error) => {
                let _ = writeln!(out, "  打开失败：{error}");
                let _ = writeln!(out, "  [阻塞] 会话数据库无法打开");
                blocking += 1;
            }
        }
    } else {
        let _ = writeln!(out, "  尚无记录（首次启动会创建）");
    }
    let _ = writeln!(out);

    // —— 音频设备 ——
    let _ = writeln!(out, "【音频设备】");
    match hint_audio::list_input_devices() {
        Ok(devices) => {
            let _ = writeln!(out, "  输入设备：{} 个", devices.len());
            for device in devices.iter().take(8) {
                let _ = writeln!(
                    out,
                    "    - {}{}",
                    device.name,
                    if device.is_default {
                        "（默认）"
                    } else {
                        ""
                    }
                );
            }
            if devices.is_empty() {
                let _ = writeln!(out, "  [阻塞] 未检测到麦克风");
                blocking += 1;
            }
        }
        Err(error) => {
            let _ = writeln!(out, "  输入设备枚举失败：{error}");
            let _ = writeln!(out, "  [阻塞] 无法枚举麦克风");
            blocking += 1;
        }
    }
    match hint_audio::list_output_devices() {
        Ok(devices) => {
            let _ = writeln!(out, "  输出设备：{} 个", devices.len());
            for device in devices.iter().take(8) {
                let _ = writeln!(
                    out,
                    "    - {}{}",
                    device.name,
                    if device.is_default {
                        "（默认）"
                    } else {
                        ""
                    }
                );
            }
            if devices.is_empty() {
                let _ = writeln!(out, "  [阻塞] 未检测到系统输出设备，听不到面试官");
                blocking += 1;
            }
        }
        Err(error) => {
            let _ = writeln!(out, "  输出设备枚举失败：{error}");
            let _ = writeln!(out, "  [阻塞] 无法枚举输出设备");
            blocking += 1;
        }
    }
    let _ = writeln!(out);

    // —— 最近日志 ——
    let _ = writeln!(out, "【最近日志】");
    let log_path = app_data_dir.join("logs").join("hint.log");
    let _ = writeln!(out, "  文件：{}", log_path.display());
    match tail_warnings(&log_path, LOG_TAIL_LINES) {
        Ok(lines) if lines.is_empty() => {
            let _ = writeln!(out, "  最近 {LOG_TAIL_LINES} 行内没有 WARN/ERROR");
        }
        Ok(lines) => {
            let _ = writeln!(out, "  最近 {} 条 WARN/ERROR：", lines.len());
            for line in lines {
                let _ = writeln!(out, "    {line}");
            }
        }
        Err(message) => {
            let _ = writeln!(out, "  读取失败：{message}");
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "结论：{}",
        if blocking == 0 {
            "未发现阻塞项。若仍是「没有声音 / 答不出来」，请连同本报告与 logs/hint.log 一起反馈。"
                .to_string()
        } else {
            format!("发现 {blocking} 个阻塞项，见上文 [阻塞] 行。")
        }
    );
    out
}

/// 从日志尾部取 WARN/ERROR 行。
///
/// 筛选与截断规则在 `hint_core::diagnostics::tail_warnings`（那边有单测，
/// 壳层这边只在 CI 上过 clippy/编译，测不了）。
fn tail_warnings(path: &Path, max_lines: usize) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    Ok(hint_core::diagnostics::tail_warnings(&text, max_lines))
}

/// 尝试把输出挂到调用方控制台。
///
/// release 构建带 `windows_subsystem = "windows"`，默认没有控制台，
/// `println!` 会静默丢失。从 cmd / PowerShell 运行时父进程是有控制台的，
/// `AttachConsole(ATTACH_PARENT_PROCESS)` 就能把输出接管过去。
/// 失败不算错误：报告文件仍会写盘。
#[cfg(windows)]
fn attach_parent_console() -> bool {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
    }
    // SAFETY: 只调用无副作用的控制台附加 API，不需要释放资源。
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

#[cfg(not(windows))]
fn attach_parent_console() -> bool {
    true
}

/// 供 `--doctor` 探测模型根目录：与主程序同一规则，但**不触发迁移**。
///
/// 抽出来是因为诊断要在不启动 Tauri 的前提下跑，且必须是只读的——
/// 收集信息时顺手搬 241 MB 会让「诊断」变成有副作用的操作。
pub(crate) fn model_root_for_report(app_data_dir: &Path) -> PathBuf {
    crate::model_root_path(app_data_dir).unwrap_or_else(|| app_data_dir.join("voice"))
}
