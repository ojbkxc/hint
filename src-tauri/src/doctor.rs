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
    // 全新机器（或刚被卸载重装、数据目录被清）上 app_data_dir 还不存在，
    // 直接 write 会失败——而 `--doctor` 恰恰最该在这种机器上可用。
    // 先建目录，失败才如实报错。
    let dir_ready = std::fs::create_dir_all(app_data_dir).is_ok();
    let saved = dir_ready && std::fs::write(&path, &report).is_ok();
    if saved {
        println!("\n报告已保存：{}", path.display());
    } else {
        eprintln!("\n报告写入失败（目录不可写）：{}", path.display());
    }
    if report.contains("[阻塞]") { 1 } else { 0 }
}

/// 收集全部诊断信息，返回可直接打印的文本。
///
/// pub(crate)：GUI 的「环境自检」也用它，保证屏幕上看到的内容与
/// --doctor 写进报告的文件完全一致——两处各写一份判据是最容易漂的。
pub(crate) fn collect(app_data_dir: &Path) -> String {
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
    // 状态里已经带了安装目录，不再单独打一行免得两份路径看着像不一致。
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
    // 区分「空壳目录」（迁移成功后的残留，删了无损失）与「仍有模型」（迁移
    // 没跑或失败，里面可能还有 241 MB），两种情况该说的话不一样。
    let legacy = app_data_dir.join("voice");
    if legacy.is_dir() && legacy != model_root {
        let legacy_model_dir = legacy.join(hint_stt::assets::DIR_NAME);
        let leftover_bytes = if legacy_model_dir.is_dir() {
            dir_size(&legacy_model_dir)
        } else {
            0
        };
        if leftover_bytes > 0 {
            let _ = writeln!(
                out,
                "  提示：旧模型目录仍有数据（{}，{}），当前使用的是新位置；确认新位置可用后可删除旧目录",
                legacy.display(),
                hint_core::diagnostics::format_mb(leftover_bytes)
            );
        } else {
            let _ = writeln!(
                out,
                "  提示：旧模型目录只剩空壳（{}），可安全删除",
                legacy.display()
            );
        }
    }
    let _ = writeln!(out);

    // —— 配置与凭据 ——
    let settings_path = app_data_dir.join("settings.json");
    // 用 load_reporting 而不是 load：`load` 在配置损坏时会**静默**把文件改名成
    // `.json.bad` 并回退默认值，诊断报告里就只剩「协议：openai / Base URL：未填写」
    // ——看起来像用户没配过，实际是他填的简历、项目、接口全被重置了。
    // 这类「报告本身看不出异常」的情况必须显式点出来。
    let (settings, settings_corrupted) =
        hint_core::settings::Settings::load_reporting(&settings_path);
    let _ = writeln!(out, "【配置】");
    let _ = writeln!(
        out,
        "  配置文件：{}{}",
        settings_path.display(),
        if settings_path.exists() {
            ""
        } else if settings_corrupted {
            "（损坏，已备份为 settings.json.bad，当前是默认值）"
        } else {
            "（不存在，使用默认值）"
        }
    );
    if settings_corrupted {
        let _ = writeln!(
            out,
            "  [阻塞] 配置文件损坏，已回退默认值；简历与项目资料需重新填写（原文件已备份为 settings.json.bad）"
        );
        blocking += 1;
    }
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
        "  浮层字号：{:.0}%",
        settings.overlay_font_scale * 100.0
    );
    // 内容保护直接决定「共享屏幕时对方能不能看到浮层」，是用户最关心的
    // 一项，必须能在报告里自查；同时点明生效的系统门槛。
    let _ = writeln!(
        out,
        "  浮层内容保护（共享屏幕时隐藏）：{}",
        if settings.overlay_content_protected {
            "开（Win10 2004+ 生效）"
        } else {
            "关（共享屏幕时对方能看到浮层）"
        }
    );
    // 代理只报模式与「是否填了地址」，不打印地址本身：代理 URL 可能带
    // 用户名密码（user:pass@host），报告是随手转发的东西。
    let proxy_mode = settings.proxy_mode.as_str();
    let proxy_desc = match proxy_mode {
        "manual" => {
            if settings.proxy_url.trim().is_empty() {
                "手动指定（但地址为空，等同直连）"
            } else {
                "手动指定（地址已填写）"
            }
        }
        "system" => "跟随系统",
        _ => "直连",
    };
    let _ = writeln!(out, "  出网代理：{proxy_desc}");
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
            // 「文件不存在」是全新安装的正常状态，不是错误——报「读取失败」
            // 会让用户以为程序坏了（CI 上的冒烟测试正是被这句话误导过一次）。
            if log_path.exists() {
                let _ = writeln!(out, "  读取失败：{message}");
            } else {
                let _ = writeln!(out, "  尚无日志文件（首次启动后会创建）");
            }
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

/// 目录下所有文件的字节总数（不跟随符号链接，读不到就按 0 算）。
///
/// 只用于报告里「旧目录还剩多少」这一句，精度不重要、绝不能因为权限问题中断诊断。
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            total = total.saturating_add(dir_size(&entry.path()));
        }
    }
    total
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
/// release 构建带 `windows_subsystem = "windows"`，**进程启动时没有控制台**，
/// 进程的 stdout/stderr 句柄是无效的（指向 NUL 或干脆为 NULL）。这时只调
/// `AttachConsole(ATTACH_PARENT_PROCESS)` 是不够的——控制台附加上了，但
/// CRITICAL 的是：Rust 的 `std::io::stdout()` 仍然指向那个无效句柄，
/// `print!` 继续静默丢弃。实测（1.0.19 真机，cmd 与 PowerShell 都试过）
/// 输出恒为空，报告文件能写。所以必须再 `CreateFileW("CONOUT$")` 拿到
/// 新控制台的句柄并用 `SetStdHandle` 重新绑定。
///
/// 失败不算错误：报告文件仍会写盘，用户至少有文件可看。
#[cfg(windows)]
fn attach_parent_console() -> bool {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *mut core::ffi::c_void,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: *mut core::ffi::c_void,
        ) -> *mut core::ffi::c_void;
        fn SetStdHandle(nStdHandle: u32, hHandle: *mut core::ffi::c_void) -> i32;
        fn GetStdHandle(nStdHandle: u32) -> *mut core::ffi::c_void;
    }

    // SAFETY: 全部是 Win32 控制台句柄操作；句柄由系统释放（进程退出时），
    // 这里不要手动 CloseHandle——之后 stdout 还要用它写数据。
    unsafe {
        // 已经是控制台进程（比如从 cmd 直接跑 debug 版）就别重复附加。
        let already = !GetStdHandle(STD_OUTPUT_HANDLE).is_null();
        if !already {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        // "CONOUT$" 的 UTF-16 字面量（含结尾 NUL）。
        let conout: [u16; 8] = [
            b'C' as u16,
            b'O' as u16,
            b'N' as u16,
            b'O' as u16,
            b'U' as u16,
            b'T' as u16,
            b'$' as u16,
            0,
        ];
        let handle = CreateFileW(
            conout.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        );
        // INVALID_HANDLE_VALUE 是 -1。
        if handle.is_null() || handle as isize == -1 {
            return false;
        }
        // 两个流都要绑：诊断报告走 stdout，失败提示走 stderr。
        let ok_out = SetStdHandle(STD_OUTPUT_HANDLE, handle) != 0;
        let ok_err = SetStdHandle(STD_ERROR_HANDLE, handle) != 0;
        ok_out && ok_err
    }
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
