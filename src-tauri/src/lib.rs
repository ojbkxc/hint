//! hint 的 Tauri 壳：窗口、托盘、全局热键、命令注册。
//!
//! 业务链路在 [`session`]（采集 → STT → 会话存储 → LLM 流式）与
//! [`commands`]（前端 invoke 入口）里。

mod commands;
mod import;
mod logging;
mod session;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use hint_core::session::LifecycleQueue;
use hint_store::Store;
use hint_stt::ModelManager;
use tauri::{
    Manager, RunEvent, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// 忽略锁中毒：任何线程 panic 都不该让命令层跟着挂。
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 运行中的会话：命令层只读取是否存在，结束时由 `session` 模块整体取走。
pub(crate) struct RunningSession {
    pub(crate) shared: Arc<session::SessionShared>,
    pub(crate) handle: hint_audio::AudioHandle,
    pub(crate) jobs: Option<std::sync::mpsc::SyncSender<session::SttJob>>,
    pub(crate) dispatcher: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct AppState {
    pub(crate) settings_path: PathBuf,
    /// 日志目录，`open_log_dir` 命令用。
    pub(crate) log_dir: PathBuf,
    /// 首次关闭主窗口的托盘提示只发一次。
    pub(crate) close_tip_shown: AtomicBool,
    /// 会话数据库；`hint_store::Store` 不是 `Sync`，统一用 `Mutex` 保护。
    pub(crate) store: Mutex<Store>,
    /// 语音模型管理器（内部状态全 `Arc` 共享，可克隆）。
    pub(crate) model: ModelManager,
    /// 正在运行的会话。
    pub(crate) session: Mutex<Option<RunningSession>>,
    /// 模型下载的取消标志；`None` 表示没有进行中的下载。
    pub(crate) prepare_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// 最近一次会话的分发线程完成信号；会话结束后仍保留，供 `remove_model` 等待。
    pub(crate) dispatch_done: Mutex<Option<Arc<(Mutex<bool>, Condvar)>>>,
    /// 会话生命周期队列：串行化 start/stop，消除 check-exec 竞态。
    pub(crate) lifecycle: Mutex<LifecycleQueue>,
}

/// 托盘里的「开始/结束面试」菜单项，随会话状态启用/禁用。
pub(crate) struct TrayItems {
    pub(crate) session_start: MenuItem<tauri::Wry>,
    pub(crate) session_stop: MenuItem<tauri::Wry>,
    /// 托盘图标本身：会话状态变化时更新 tooltip，用户悬停即可知道是否在面试。
    pub(crate) tray: tauri::tray::TrayIcon<tauri::Wry>,
}

/// 全局热键的 4 组组合（与前端约定一致）。
const SHORTCUTS: [(&str, Code); 4] = [
    ("ctrl+shift+space", Code::Space),
    ("ctrl+shift+a", Code::KeyA),
    ("ctrl+shift+r", Code::KeyR),
    ("ctrl+shift+s", Code::KeyS),
];

/// 托盘 tooltip：主窗口关掉后，这是用户唯一能确认「程序在干什么」的地方。
const TRAY_TOOLTIP_IDLE: &str = "hint · 空闲（右键可退出；Ctrl+Shift+Space 显示浮层）";
const TRAY_TOOLTIP_LIVE: &str = "hint · 面试进行中（右键可结束或退出）";

fn combo(code: Code) -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), code)
}

impl AppState {
    fn new(data_dir: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|error| format!("创建数据目录失败：{error}"))?;
        let db_path = data_dir.join("hint.db");
        let store = match Store::open(&db_path) {
            Ok(store) => store,
            Err(error) => {
                // 磁盘异常时退回内存库，保证应用仍能启动（重启后会话记录丢失）。
                tracing::error!(%error, path = %db_path.display(), "打开会话数据库失败，改用内存库");
                Store::open_in_memory().map_err(|fallback| {
                    format!("会话数据库不可用：{error}；内存库也失败：{fallback}")
                })?
            }
        };
        // 保留最近 30 天的会话记录；清理失败不影响启动。
        match store.cleanup_older_than_days(30) {
            Ok(removed) if removed > 0 => {
                tracing::info!(removed, "已清理 30 天前的会话记录");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "清理旧会话记录失败");
            }
        }
        Ok(Self {
            settings_path: data_dir.join("settings.json"),
            log_dir: data_dir.join("logs"),
            close_tip_shown: AtomicBool::new(false),
            store: Mutex::new(store),
            model: ModelManager::new(data_dir.join("voice")),
            session: Mutex::new(None),
            prepare_cancel: Mutex::new(None),
            dispatch_done: Mutex::new(None),
            lifecycle: Mutex::new(LifecycleQueue::default()),
        })
    }
}

fn toggle_overlay(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.set_focusable(false);
        let _ = window.hide();
    } else {
        let _ = window.show();
        // 浮层配置为 focus:false；可聚焦才能收到 Esc / 方向键。
        let _ = window.set_focusable(true);
        let _ = window.set_focus();
    }
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 托盘菜单的「开始/结束面试」开关。
pub(crate) fn set_session_running(app: &tauri::AppHandle, running: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.session_start.set_enabled(!running);
        let _ = items.session_stop.set_enabled(running);
        // 用户关掉主窗口后只剩托盘可见，tooltip 必须能反映会话是否在跑。
        let _ = items.tray.set_tooltip(Some(if running {
            TRAY_TOOLTIP_LIVE
        } else {
            TRAY_TOOLTIP_IDLE
        }));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 日志初始化推迟到 setup：那时才拿到 app_data_dir，才能落盘。
    tauri::Builder::default()
        // 单实例保护必须是第一个 plugin：第二个实例把参数交给首实例后直接退出。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let log_dir = data_dir.join("logs");
            logging::init(&log_dir);
            match AppState::new(&data_dir) {
                Ok(state) => {
                    app.manage(state);
                }
                Err(error) => {
                    tracing::error!(%error, "初始化应用状态失败");
                    return Err(error.into());
                }
            }
            session::emit_status(app.handle(), "idle", "等待连接");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // 关闭窗口 = 收进托盘，不退出应用；所有窗口都拦，避免浮层 /
                // 截图窗口被销毁后 `get_webview_window` 再也拿不到。
                api.prevent_close();
                let _ = window.hide();
                // 用户很容易以为点了 × 就是退出，第一次关闭主窗口时明确告知。
                if window.label() == "main"
                    && let Some(state) = window.try_state::<AppState>()
                    && !state.close_tip_shown.swap(true, Ordering::AcqRel)
                {
                    session::emit_notice(
                        window.app_handle(),
                        "info",
                        "窗口已隐藏到系统托盘；右键托盘图标可退出，按 Ctrl+Shift+Space 显示浮层（左键托盘图标可恢复主窗口）。",
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::save_api_key,
            commands::has_api_key,
            commands::test_llm,
            commands::fetch_models,
            commands::import_document_text,
            commands::start_session,
            commands::stop_session,
            commands::toggle_session,
            commands::list_sessions,
            commands::session_detail,
            commands::export_session_markdown,
            commands::delete_session,
            commands::delete_all_sessions,
            commands::hide_overlay,
            commands::show_overlay,
            commands::toggle_overlay,
            commands::regenerate_answer,
            commands::list_audio_devices,
            commands::prepare_model,
            commands::model_status,
            commands::cancel_prepare,
            commands::remove_model,
            commands::start_capture,
            commands::cancel_capture,
            commands::finish_capture,
            commands::open_log_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| match event {
            RunEvent::Ready => {
                if let Err(error) = setup_after_ready(app) {
                    // 热键注册失败已在 register_shortcuts 里单独提示，
                    // 走到这里只可能是托盘没建起来，按托盘错误上报。
                    tracing::error!(%error, "托盘初始化失败");
                    session::emit_notice(app, "error", format!("托盘初始化失败：{error}"));
                }
            }
            RunEvent::ExitRequested { .. } => session::shutdown(app),
            // 托盘「退出」走 `app.exit(0)`，不保证经过 ExitRequested；
            // 这里再兜一次（stop_session 幂等，没有会话时直接返回 Err 被忽略）。
            RunEvent::Exit => session::shutdown(app),
            _ => {}
        });
}

/// `RunEvent::Ready` 在主线程上触发，`Menu` / 热键注册这类要求主线程的
/// API 可以同步完成。
fn setup_after_ready(app: &tauri::AppHandle) -> tauri::Result<()> {
    // 托盘与热键互不依赖：托盘构建失败（图标 / 菜单资源异常）不能让热键一起
    // 被跳过——主窗口关掉后，Ctrl+Shift+Space 是用户唯一能取回浮层的入口。
    let tray = build_tray(app);
    register_shortcuts(app);
    tray?;
    if app.get_webview_window("overlay").is_none() {
        tracing::warn!("浮层窗口未创建，请检查 tauri.conf.json");
    }
    Ok(())
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    if app.try_state::<TrayItems>().is_some() {
        return Ok(());
    }
    let settings_item = MenuItem::with_id(app, "settings", "打开设置", true, None::<&str>)?;
    let overlay_item = MenuItem::with_id(app, "overlay", "显示 / 隐藏浮层", true, None::<&str>)?;
    let session_start = MenuItem::with_id(app, "session-start", "开始面试", true, None::<&str>)?;
    let session_stop = MenuItem::with_id(app, "session-stop", "结束面试", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &settings_item,
            &overlay_item,
            &separator,
            &session_start,
            &session_stop,
            &separator,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .cloned()
                .unwrap_or_else(|| tauri::image::Image::new_owned(vec![0u8; 4], 1, 1)),
        )
        .tooltip(TRAY_TOOLTIP_IDLE)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_main(app),
            "overlay" => toggle_overlay(app),
            "session-start" => session::spawn_start(app),
            "session-stop" => session::spawn_stop(app),
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
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayItems {
        session_start: session_start.clone(),
        session_stop: session_stop.clone(),
        tray,
    });
    Ok(())
}

fn register_shortcuts(app: &tauri::AppHandle) {
    let accelerators: Vec<&str> = SHORTCUTS
        .iter()
        .map(|(accelerator, _)| *accelerator)
        .collect();
    let result = app
        .global_shortcut()
        .on_shortcuts(accelerators, |app, shortcut, event| {
            if event.state == ShortcutState::Pressed {
                handle_pressed_shortcut(app, *shortcut);
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "全局热键注册失败");
        commands::emit_hotkey_failure(app, &error.to_string());
    }
}

fn handle_pressed_shortcut(app: &tauri::AppHandle, shortcut: Shortcut) {
    if shortcut == combo(Code::Space) {
        toggle_overlay(app);
    } else if shortcut == combo(Code::KeyA) {
        session::spawn_regenerate(app);
    } else if shortcut == combo(Code::KeyR) {
        // 与设置页/命令层的切换一致：方向在生命周期队列锁内决定，
        // 启动中再按一次会排队停止，而不是被合并成一个无效的 Start。
        session::spawn_toggle(app);
    } else if shortcut == combo(Code::KeyS) {
        commands::emit_capture_placeholder(app);
    }
}
