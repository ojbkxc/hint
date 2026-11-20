//! Tauri 命令层。所有命令统一返回 `Result<_, String>`，绝不 panic。
//!
//! 规则：async 命令里不长时间持锁；STT / 下载等阻塞 API 一律走
//! `spawn_blocking`；事件发送失败只记日志。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hint_core::session::LifecycleDir;
use hint_core::{llm, secret, settings::Settings};
use hint_stt::download::user_message_for;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::session::{self, EVT_MODEL_PROGRESS};
use crate::{AppState, lock};

pub(crate) const CAPTURE_WINDOW: &str = "capture";

// ---------------------------------------------------------------------------
// 设置与凭证（M0，保持不变）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(Settings::load(&state.settings_path))
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    settings
        .save(&state.settings_path)
        .map_err(|error| format!("保存配置失败：{error}"))?;
    // clamp 只用于事件值：手改配置文件写入越界值时，浮层也不会变成完全不可见。
    // 不回写配置文件，避免静默修改用户数据。
    let opacity = settings.overlay_opacity.clamp(0.2, 1.0);
    session::emit(
        &app,
        session::EVT_OVERLAY_OPACITY,
        json!({ "opacity": opacity }),
    );
    Ok(())
}

#[tauri::command]
pub fn save_api_key(provider: String, key: String) -> Result<(), String> {
    secret::save(&provider, &key).map_err(|error| format!("写入 Windows 凭据失败：{error}"))
}

#[tauri::command]
pub fn has_api_key(provider: String) -> Result<bool, String> {
    Ok(secret::has(&provider))
}

/// 解析本次请求要用的 API Key：优先用表单里未保存的 Key，否则读凭据管理器。
///
/// 读不到时返回调用方提供的中文提示，避免把凭据管理器的内部错误暴露给用户。
fn resolve_api_key(
    api_key: Option<String>,
    provider: &str,
    missing_hint: &str,
) -> Result<String, String> {
    match api_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
    {
        Some(key) => Ok(key),
        None => secret::get(provider).map_err(|_| missing_hint.to_string()),
    }
}

#[tauri::command]
pub async fn test_llm(
    state: State<'_, AppState>,
    prompt: String,
    settings: Option<Settings>,
    api_key: Option<String>,
) -> Result<llm::ChatResponse, String> {
    // 设置页未保存时可以用当前表单直接测试；不传则沿用已保存配置。
    let settings = settings.unwrap_or_else(|| Settings::load(&state.settings_path));
    let provider = settings.provider.as_str().to_string();
    // 表单里刚输入、还没点「保存」的 Key 也能直接用于测试。
    let key = resolve_api_key(
        api_key,
        &provider,
        "当前接口还没有保存 API Key：请在上方输入框粘贴 Key 后点「保存」，\
         或直接点「测试连接」自动保存并测试",
    )?;
    let system = settings.system_prompt();
    llm::complete(
        &settings,
        &key,
        llm::ChatRequest {
            system,
            user: prompt,
            ..llm::ChatRequest::default()
        },
    )
    .await
    .map_err(|error| format!("模型连接失败：{error}"))
}

/// 拉取接口可用的模型列表（设置页下拉选择用）。
///
/// 与 `test_llm` 同理：支持用表单里未保存的 Key；未保存时给出中文提示，
/// 不把凭据管理器的内部错误暴露给用户。
#[tauri::command]
pub async fn fetch_models(
    state: State<'_, AppState>,
    settings: Option<Settings>,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let settings = settings.unwrap_or_else(|| Settings::load(&state.settings_path));
    let provider = settings.provider.as_str().to_string();
    let key = resolve_api_key(
        api_key,
        &provider,
        "当前接口还没有保存 API Key：请先保存 Key，再拉取模型列表",
    )?;
    llm::fetch_models(&settings, &key)
        .await
        .map_err(|error| format!("拉取模型列表失败：{error}"))
}

/// 从本地文件提取资料文本（TXT / MD / DOCX / PDF）。
#[tauri::command]
pub async fn import_document_text(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || crate::import::document_text(&path))
        .await
        .map_err(|error| format!("导入任务失败：{error}"))?
}

// ---------------------------------------------------------------------------
// 会话
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_session(app: AppHandle) -> Result<i64, String> {
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (_, result) = session::submit_lifecycle(&task_app, LifecycleDir::Start);
        result
    })
    .await
    .map_err(|error| format!("启动会话任务失败：{error}"))?
}

#[tauri::command]
pub async fn stop_session(app: AppHandle) -> Result<(), String> {
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (_, result) = session::submit_lifecycle(&task_app, LifecycleDir::Stop);
        result.map(|_| ())
    })
    .await
    .map_err(|error| format!("结束会话任务失败：{error}"))?
}

#[tauri::command]
pub async fn toggle_session(app: AppHandle) -> Result<bool, String> {
    // 方向在生命周期队列的锁内决定，再串行执行；启动中/已运行 → Stop，
    // 空闲/正在停止 → Start。这样在加载模型的 10–30 秒里再按一次也能排上停止。
    let task_app = app.clone();
    let dir = tauri::async_runtime::spawn_blocking(move || {
        let (dir, _, _) = session::submit_toggle(&task_app);
        dir
    })
    .await
    .map_err(|error| format!("切换会话任务失败：{error}"))?;
    Ok(dir == LifecycleDir::Start)
}

fn format_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

/// 会话列表：拍平成前端 `SessionSummary` 的形状。
#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let store = lock(&state.store);
    let summaries = store
        .list_sessions(200)
        .map_err(|error| format!("读取会话列表失败：{error}"))?;
    Ok(summaries
        .into_iter()
        .map(|summary| {
            json!({
                "id": summary.session.id.to_string(),
                "startedAt": format_time(summary.session.started_at),
                "endedAt": summary.session.ended_at.map(format_time),
                "model": summary.session.model_id,
                "provider": summary.session.provider_id,
                "note": summary.session.note.unwrap_or_default(),
                "segmentCount": summary.segment_count,
                "answerCount": summary.answer_count,
            })
        })
        .collect())
}

/// 会话详情：通道名映射为前端约定的 `me` / `interviewer`。
#[tauri::command]
pub fn session_detail(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let session_id = parse_id(&id)?;
    let store = lock(&state.store);
    let detail = store
        .session_detail(session_id)
        .map_err(|error| format!("读取会话详情失败：{error}"))?;
    let segments: Vec<Value> = detail
        .segments
        .into_iter()
        .map(|segment| {
            json!({
                "id": segment.id.to_string(),
                "channel": if segment.channel == hint_store::CHANNEL_MIC { "me" } else { "interviewer" },
                "text": segment.text,
                "isEcho": segment.is_echo,
                "startMs": segment.start_ms,
                "endMs": segment.end_ms,
            })
        })
        .collect();
    let answers: Vec<Value> = detail
        .answers
        .into_iter()
        .map(|answer| {
            json!({
                "id": answer.id.to_string(),
                "text": answer.text,
                "model": answer.model_id,
                "ttftMs": answer.ttft_ms,
                "totalMs": answer.total_ms,
            })
        })
        .collect();
    Ok(json!({
        "id": session_id.to_string(),
        "startedAt": format_time(detail.session.started_at),
        "endedAt": detail.session.ended_at.map(format_time),
        "model": detail.session.model_id,
        "segments": segments,
        "answers": answers,
    }))
}

#[tauri::command]
pub fn export_session_markdown(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let session_id = parse_id(&id)?;
    let store = lock(&state.store);
    store
        .export_markdown(session_id)
        .map_err(|error| format!("导出失败：{error}"))
}

#[tauri::command]
pub fn delete_all_sessions(state: State<'_, AppState>, app: AppHandle) -> Result<usize, String> {
    // 启动中的会话记录已经落库，但 state.session 仍为空；此时清空会把
    // 正在启动的会话一起删掉，后续写入全部失败。
    if session::is_busy(&app) {
        return Err("会话正在启动或进行中，结束后再清空会话记录。".to_string());
    }
    let store = lock(&state.store);
    store
        .delete_all_sessions()
        .map_err(|error| format!("清空会话记录失败：{error}"))
}

#[tauri::command]
pub fn delete_session(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<bool, String> {
    let session_id = parse_id(&id)?;
    if session::is_busy(&app) {
        return Err("会话正在启动或进行中，结束后再删除记录。".to_string());
    }
    let store = lock(&state.store);
    store
        .delete_session(session_id)
        .map_err(|error| format!("删除会话失败：{error}"))
}

fn parse_id(raw: &str) -> Result<i64, String> {
    raw.trim()
        .parse::<i64>()
        .map_err(|_| format!("无效的会话编号：{raw}"))
}

// ---------------------------------------------------------------------------
// 浮层
// ---------------------------------------------------------------------------

fn overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("overlay")
        .ok_or_else(|| "浮层窗口不存在。".to_string())
}

fn show_overlay_window(window: &WebviewWindow) -> Result<(), String> {
    window
        .show()
        .map_err(|error| format!("显示浮层失败：{error}"))?;
    // 浮层平时不可聚焦；显示后再打开，Esc / 方向键才收得到。
    let _ = window.set_focusable(true);
    let _ = window.set_focus();
    Ok(())
}

fn hide_overlay_window(window: &WebviewWindow) -> Result<(), String> {
    let _ = window.set_focusable(false);
    window
        .hide()
        .map_err(|error| format!("隐藏浮层失败：{error}"))
}

#[tauri::command]
pub fn hide_overlay(app: AppHandle) -> Result<(), String> {
    let window = overlay_window(&app)?;
    hide_overlay_window(&window)
}

#[tauri::command]
pub fn show_overlay(app: AppHandle) -> Result<(), String> {
    let window = overlay_window(&app)?;
    show_overlay_window(&window)
}

#[tauri::command]
pub fn toggle_overlay(app: AppHandle) -> Result<bool, String> {
    let window = overlay_window(&app)?;
    if window.is_visible().unwrap_or(false) {
        hide_overlay_window(&window)?;
        Ok(false)
    } else {
        show_overlay_window(&window)?;
        Ok(true)
    }
}

#[tauri::command]
pub async fn regenerate_answer(app: AppHandle) -> Result<(), String> {
    session::regenerate(&app)
}

/// 打开日志目录，便于用户把 `hint.log` 发给支持。
#[tauri::command]
pub fn open_log_dir(state: State<'_, AppState>) -> Result<(), String> {
    let dir = &state.log_dir;
    if !dir.is_dir() {
        return Err(format!("日志目录尚未创建：{}", dir.display()));
    }
    std::process::Command::new("explorer")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开日志目录（{}）：{error}", dir.display()))
}

// ---------------------------------------------------------------------------
// 设备
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_audio_devices() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let inputs = hint_audio::list_input_devices()
            .map_err(|error| format!("枚举输入设备失败：{error}"))?;
        let outputs = hint_audio::list_output_devices()
            .map_err(|error| format!("枚举输出设备失败：{error}"))?;
        Ok(json!({
            "inputs": inputs
                .into_iter()
                .map(|device| json!({ "id": device.id, "name": device.name, "isDefault": device.is_default }))
                .collect::<Vec<_>>(),
            "outputs": outputs
                .into_iter()
                .map(|device| json!({ "id": device.id, "name": device.name, "isDefault": device.is_default }))
                .collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(|error| format!("枚举设备任务失败：{error}"))?
}

// ---------------------------------------------------------------------------
// 语音模型下载
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn model_status(state: State<'_, AppState>) -> Result<Value, String> {
    let status = state.model.status();
    serde_json::to_value(status).map_err(|error| format!("序列化模型状态失败：{error}"))
}

/// 下载期间在 `AppState::prepare_cancel` 里占位；Drop 时清空。
///
/// 只清自己的占位：取消后用户立刻重新下载时，新任务的标志可能已经写进槽位，
/// 无条件清空会把新任务的取消标志一起抹掉（之后无法取消，还会允许并发下载）。
struct PrepareGuard {
    app: AppHandle,
    flag: Arc<AtomicBool>,
}

impl Drop for PrepareGuard {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        let mut slot = lock(&state.prepare_cancel);
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.flag))
        {
            *slot = None;
        }
    }
}

#[tauri::command]
pub async fn prepare_model(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let cancel = {
        let mut slot = lock(&state.prepare_cancel);
        // 取消只置位、不腾槽：旧任务真正退出（PrepareGuard 析构）前，
        // 不允许第二个下载并发写同一批 .part 文件。
        if slot.is_some() {
            return Err("模型下载已在进行中。".to_string());
        }
        let flag = Arc::new(AtomicBool::new(false));
        *slot = Some(Arc::clone(&flag));
        flag
    };
    let _guard = PrepareGuard {
        app: app.clone(),
        flag: Arc::clone(&cancel),
    };

    let manager = state.model.clone();
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        manager.prepare(
            move |progress| {
                session::emit(
                    &task_app,
                    EVT_MODEL_PROGRESS,
                    json!({
                        "step": progress.step.label(),
                        "resource": progress.resource,
                        "completedBytes": progress.completed_bytes,
                        "totalBytes": progress.total_bytes,
                        "source": progress.source,
                    }),
                );
            },
            &cancel,
        )
    })
    .await
    .map_err(|error| format!("模型下载任务失败：{error}"))?;

    match result {
        Ok(()) => {
            session::emit_notice(&app, "info", "语音模型已就绪");
            Ok(true)
        }
        // 用户主动取消不是失败：给普通提示，不要把「已取消」显示成错误。
        // 返回 false 让前端区分「检查完成」与「用户取消」。
        Err(error) if error.is_cancelled() => {
            session::emit_notice(&app, "info", "已取消语音模型下载");
            Ok(false)
        }
        Err(error) => {
            let message = user_message_for(&error);
            session::emit_notice(&app, "error", format!("模型下载失败：{message}"));
            Err(format!("模型下载失败：{message}"))
        }
    }
}

#[tauri::command]
pub fn cancel_prepare(state: State<'_, AppState>) -> Result<bool, String> {
    // 只置位、不腾槽：槽位由 PrepareGuard 在旧任务真正退出后清理，
    // 避免「取消后立刻再点下载」时两个任务并发写同一批 .part 文件。
    let slot = lock(&state.prepare_cancel);
    match slot.as_ref() {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn remove_model(app: AppHandle) -> Result<(), String> {
    // is_busy 同时覆盖「启动中」：模型正在被 worker 加载时删文件会留下半删状态。
    if session::is_busy(&app) {
        return Err("会话正在启动或进行中，结束后再删除模型。".to_string());
    }
    let (manager, done) = {
        let state = app.state::<AppState>();
        if lock(&state.prepare_cancel).is_some() {
            return Err("模型正在下载，请先取消下载再删除。".to_string());
        }
        (state.model.clone(), lock(&state.dispatch_done).clone())
    };
    tauri::async_runtime::spawn_blocking(move || {
        // 刚结束的会话可能还有 STT worker 进程在退出，等分发线程发出完成信号。
        if let Some(done) = done {
            let (done_lock, done_cv) = &*done;
            let mut finished = lock(done_lock);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !*finished {
                let now = std::time::Instant::now();
                if now >= deadline {
                    return Err("语音引擎仍在收尾，请稍后重试。".to_string());
                }
                let (next, _) = done_cv
                    .wait_timeout(finished, deadline - now)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                finished = next;
            }
        }
        manager
            .remove()
            .map_err(|error| format!("删除模型失败：{}", user_message_for(&error)))
    })
    .await
    .map_err(|error| format!("删除模型任务失败：{error}"))?
}

// ---------------------------------------------------------------------------
// 截图选区（M5 预留）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn start_capture(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(CAPTURE_WINDOW)
        .ok_or_else(|| "截图窗口未创建，请检查 tauri.conf.json。".to_string())?;
    let _ = window.set_fullscreen(true);
    window
        .show()
        .map_err(|error| format!("显示截图窗口失败：{error}"))?;
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command]
pub fn cancel_capture(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(CAPTURE_WINDOW) {
        let _ = window.set_fullscreen(false);
        let _ = window.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn finish_capture(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(CAPTURE_WINDOW) {
        let _ = window.set_fullscreen(false);
        let _ = window.hide();
    }
    // M5：这里接截屏 + 多模态问答。先把选区回报给前端，便于调试定位。
    session::emit(
        &app,
        "hint://capture-region",
        json!({ "x": x, "y": y, "width": width, "height": height }),
    );
    Err("截图问答尚未启用（M5 预留）。".to_string())
}

// ---------------------------------------------------------------------------
// 内部工具（供托盘 / 热键）
// ---------------------------------------------------------------------------

pub(crate) fn emit_capture_placeholder(app: &AppHandle) {
    session::emit(app, "hint://capture-request", json!({}));
    session::emit_notice(app, "info", "截图问答将在 M5 启用（Ctrl+Shift+S）");
}

pub(crate) fn emit_hotkey_failure(app: &AppHandle, error: &str) {
    session::emit_notice(app, "error", format!("全局热键注册失败：{error}"));
}
