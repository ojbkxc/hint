//! 大模型调用：三协议非流式 [`complete`] 与流式 [`stream`]。
//!
//! - OpenAI 兼容（DeepSeek / 通义 / 智谱 / Ollama / vLLM / one-api 等）：
//!   `POST {base}/chat/completions`，SSE 取 `choices[0].delta.content`；
//! - Anthropic Messages：`POST {base}/v1/messages`，SSE 取 `content_block_delta.delta.text`；
//! - Gemini：`POST {base}/v1beta/models/{model}:streamGenerateContent?alt=sse`，
//!   SSE 取 `candidates[0].content.parts[].text`。
//!
//! 历史消息统一按「面试官 = user 侧，我 = assistant/model 侧」映射。
//!
//! 首字前失败（连接被拒、超时、5xx、429）会自动重试一次；已经输出过文本
//! 再断开则不重试，只报「生成中断」，避免把重复内容推给用户。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{CoreError, Result};
use crate::settings::{ProviderKind, Settings};

/// 流式调用的单次尝试超时（从发起请求到收到最后一个增量）。
const STREAM_TIMEOUT: Duration = Duration::from_secs(60);

/// 首字前失败后的重试间隔。
const STREAM_RETRY_DELAY: Duration = Duration::from_millis(1_500);

/// 流式调用的最大尝试次数（首次 + 自动重试一次）。
const MAX_STREAM_ATTEMPTS: usize = 2;

/// Anthropic / Gemini 要求首条消息必须是用户侧；历史以「我」开头时补的占位。
const LEADING_USER_PLACEHOLDER: &str = "（继续之前的对话）";

/// 对话历史中的说话人。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnRole {
    /// 面试官（映射到模型侧的 user）。
    Interviewer,
    /// 候选人本人（映射到模型侧的 assistant / model）。
    Me,
}

/// 一轮历史对话。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    /// 说话人。
    pub role: TurnRole,
    /// 该轮文本。
    pub text: String,
}

/// 内联图片（base64 数据，不含 `data:` URL 前缀）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePart {
    /// MIME 类型，例如 `image/jpeg`。
    pub mime: String,
    /// base64 编码的图片字节。
    pub data_base64: String,
}

/// 一次对话请求。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    /// 系统提示词。
    pub system: String,
    /// 本轮用户问题。
    pub user: String,
    /// 历史对话，按时间正序；默认空。
    #[serde(default)]
    pub history: Vec<Turn>,
    /// 随本轮问题一起发送的图片；默认空。
    #[serde(default)]
    pub images: Vec<ImagePart>,
}

/// 非流式调用的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    /// 完整回答文本。
    pub text: String,
    /// 使用的模型名。
    pub model: String,
}

/// 流式调用的统计结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStats {
    /// 完整回答文本（增量已通过 `on_delta` 逐段送出）。
    pub text: String,
    /// 使用的模型名。
    pub model: String,
    /// 首个非空增量的耗时（毫秒）；未收到任何增量时为 0。
    pub ttft_ms: u64,
    /// 从发起到结束的总耗时（毫秒）。
    pub total_ms: u64,
}

fn base(base_url: &str) -> &str {
    base_url.trim().trim_end_matches('/')
}

/// 协议名，用于错误提示。
fn protocol_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::OpenAi => "OpenAI 兼容",
        ProviderKind::Anthropic => "Anthropic",
        ProviderKind::Gemini => "Gemini",
    }
}

/// OpenAI 兼容的 chat completions 端点。
///
/// 用户填的 Base URL 形态五花八门，这里统一兜底：
/// - `https://api.deepseek.com` → 补成 `/v1/chat/completions`；
/// - `https://api.deepseek.com/v1` → 补成 `/v1/chat/completions`；
/// - `https://gateway.example.com/v2` → 保留版本段，拼 `/chat/completions`；
/// - `https://x.com/v1/chat/completions`（完整端点）→ 原样使用；
/// - `https://x.com/chat/completions` → 原样使用。
fn openai_chat_url(base_url: &str) -> String {
    let root = base(base_url);
    if root.ends_with("/chat/completions") {
        return root.to_string();
    }
    if has_version_suffix(root) {
        return format!("{root}/chat/completions");
    }
    format!("{root}/v1/chat/completions")
}

/// 路径最后一段是否形如 `v1` / `v2` / `v1beta` 的版本段。
fn has_version_suffix(root: &str) -> bool {
    let last = root.rsplit('/').next().unwrap_or("");
    let mut chars = last.chars();
    matches!(chars.next(), Some('v' | 'V'))
        && chars.next().is_some_and(|c| c.is_ascii_digit())
}

/// Anthropic 的 Messages 端点。
///
/// 用户把 Base URL 填成 `https://api.anthropic.com/v1`（甚至完整端点）都
/// 很常见；这里去掉重复的 `/v1`，避免拼成 `/v1/v1/messages`。
fn anthropic_messages_url(base_url: &str) -> String {
    let root = base(base_url);
    if root.ends_with("/messages") {
        return root.to_string();
    }
    if root.ends_with("/v1") {
        format!("{root}/messages")
    } else {
        format!("{root}/v1/messages")
    }
}

/// Gemini 的 generateContent 端点；`stream` 决定是否用 `alt=sse`。
///
/// 同理去掉重复的 `/v1beta`；模型名里误带的 `models/` 前缀也会被剥掉。
fn gemini_url(base_url: &str, model: &str, stream: bool) -> String {
    let root = base(base_url);
    let version = if root.ends_with("/v1beta") {
        ""
    } else {
        "/v1beta"
    };
    let model = model.trim();
    let model = model.strip_prefix("models/").unwrap_or(model);
    let action = if stream {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    format!("{root}{version}/models/{model}:{action}")
}

/// 按状态码给出中文建议；正文原始片段由 [`ensure_success`] 单独附上。
fn status_advice(status: u16, settings: &Settings) -> String {
    match status {
        401 => format!(
            "API Key 无效或已过期。请到设置页重新保存 Key（当前协议：{}）。",
            protocol_label(settings.provider)
        ),
        403 => "接口无访问权限或额度不足。请检查账号状态与套餐。".to_string(),
        404 => format!(
            "接口地址或模型名不存在。请检查 Base URL 与模型名（当前：{} / {}）。",
            base(&settings.base_url),
            settings.model.trim()
        ),
        429 => "请求过于频繁或超出限额。稍等片刻后点「重新生成」重试。".to_string(),
        500..=599 => "模型服务端异常。稍等片刻后点「重新生成」重试。".to_string(),
        _ => "请求被模型接口拒绝。请检查设置页中的 Base URL、模型名与 API Key。".to_string(),
    }
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?)
}

/// 流式调用使用的常驻客户端：复用连接，不设整体超时（由 [`STREAM_TIMEOUT`] 控制）。
async fn stream_client() -> Result<&'static reqwest::Client> {
    static CELL: tokio::sync::OnceCell<reqwest::Client> = tokio::sync::OnceCell::const_new();
    CELL.get_or_try_init(|| async {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .map_err(CoreError::from)
    })
    .await
}

/// 取字符串前 `max_chars` 个字符（按字符边界截断，避免切坏 UTF-8）。
fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// 非 2xx 响应统一转成「中文建议 + 状态码 + 原始响应片段」的错误。
async fn ensure_success(
    response: reqwest::Response,
    settings: &Settings,
) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let code = status.as_u16();
    let raw = response.text().await.unwrap_or_default();
    let body = if raw.trim().is_empty() {
        "（空响应体）".to_string()
    } else {
        truncate(&raw, 500)
    };
    Err(CoreError::HttpStatus {
        status: code,
        advice: status_advice(code, settings),
        body,
    })
}

/// 解析非流式响应体；失败时给出中文说明而不是「配置解析失败」。
fn parse_json_response(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|error| {
        CoreError::Api(format!(
            "模型返回的不是合法 JSON：{error}；响应片段：{}",
            truncate(raw, 500)
        ))
    })
}

fn ensure_text(text: Option<&str>, raw: &str) -> Result<String> {
    match text {
        Some(text) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        _ => Err(CoreError::Api(format!("响应中没有可用文本：{raw}"))),
    }
}

/// 按当前配置发起一次非流式对话。
///
/// M0 的连接测试仍在使用；需要逐字输出时请改用 [`stream`]。
pub async fn complete(
    settings: &Settings,
    api_key: &str,
    request: ChatRequest,
) -> Result<ChatResponse> {
    let client = client()?;
    match settings.provider {
        ProviderKind::OpenAi => openai(&client, settings, api_key, request).await,
        ProviderKind::Anthropic => anthropic(&client, settings, api_key, request).await,
        ProviderKind::Gemini => gemini(&client, settings, api_key, request).await,
    }
}

async fn openai(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: ChatRequest,
) -> Result<ChatResponse> {
    let url = openai_chat_url(&settings.base_url);
    let body = openai_body(settings, &request, false);
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response, settings).await?;
    let raw = response.text().await?;

    let value = parse_json_response(&raw)?;
    let text = ensure_text(value["choices"][0]["message"]["content"].as_str(), &raw)?;
    Ok(ChatResponse {
        text,
        model: settings.model.clone(),
    })
}

async fn anthropic(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: ChatRequest,
) -> Result<ChatResponse> {
    let url = anthropic_messages_url(&settings.base_url);
    let body = anthropic_body(settings, &request, false);
    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response, settings).await?;
    let raw = response.text().await?;

    let value = parse_json_response(&raw)?;
    // content 里可能有 thinking / tool_use 块，取第一个 text 块。
    let text = value["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block["type"].as_str() == Some("text"))
                .and_then(|block| block["text"].as_str())
        })
        .or_else(|| value["content"][0]["text"].as_str());
    let text = ensure_text(text, &raw)?;
    Ok(ChatResponse {
        text,
        model: settings.model.clone(),
    })
}

async fn gemini(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: ChatRequest,
) -> Result<ChatResponse> {
    let url = gemini_url(&settings.base_url, &settings.model, false);
    let body = gemini_body(&request);
    let response = client
        .post(url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response, settings).await?;
    let raw = response.text().await?;

    let value = parse_json_response(&raw)?;
    let text = gemini_text_of(&value);
    let text = ensure_text(text.as_deref(), &raw)?;
    Ok(ChatResponse {
        text,
        model: settings.model.clone(),
    })
}

/// 从单个 Gemini 响应值里取出全部可见文本（跳过 thought 思维链）。
fn gemini_text_of(value: &Value) -> Option<String> {
    let parts = value["candidates"][0]["content"]["parts"].as_array()?;
    let mut text = String::new();
    for part in parts {
        if part["thought"].as_bool() == Some(true) {
            continue;
        }
        if let Some(chunk) = part["text"].as_str() {
            text.push_str(chunk);
        }
    }
    if text.is_empty() { None } else { Some(text) }
}

/// 合并连续同角色且非空的历史轮次。Anthropic 与 Gemini 都要求 user/模型侧交替。
fn merged_history(request: &ChatRequest) -> Vec<(TurnRole, String)> {
    let mut merged: Vec<(TurnRole, String)> = Vec::with_capacity(request.history.len());
    for turn in &request.history {
        if turn.text.trim().is_empty() {
            continue;
        }
        if let Some((role, text)) = merged.last_mut() {
            if *role == turn.role {
                text.push('\n');
                text.push_str(&turn.text);
                continue;
            }
        }
        merged.push((turn.role, turn.text.clone()));
    }
    merged
}

/// OpenAI 兼容协议中的角色名。
fn openai_role(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Interviewer => "user",
        TurnRole::Me => "assistant",
    }
}

/// Anthropic 协议中的角色名。
fn anthropic_role(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Interviewer => "user",
        TurnRole::Me => "assistant",
    }
}

/// Gemini 协议中的角色名。
fn gemini_role(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Interviewer => "user",
        TurnRole::Me => "model",
    }
}

/// Anthropic / Gemini 要求首条消息是用户侧；历史以「我」开头时补一条占位。
fn ensure_leading_user(messages: &mut Vec<Value>, assistant_role: &str, placeholder: Value) {
    if messages
        .first()
        .and_then(|message| message["role"].as_str())
        == Some(assistant_role)
    {
        messages.insert(0, placeholder);
    }
}

/// 组装 OpenAI 兼容协议的请求体；`stream` 控制是否要求流式返回。
fn openai_body(settings: &Settings, request: &ChatRequest, stream: bool) -> Value {
    let mut messages: Vec<Value> = Vec::with_capacity(request.history.len() + 2);
    messages.push(json!({ "role": "system", "content": request.system }));
    for (role, text) in merged_history(request) {
        messages.push(json!({ "role": openai_role(role), "content": text }));
    }

    let content = if request.images.is_empty() {
        json!(request.user)
    } else {
        let mut parts: Vec<Value> = Vec::with_capacity(request.images.len() + 1);
        parts.push(json!({ "type": "text", "text": request.user }));
        for image in &request.images {
            parts.push(json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{};base64,{}", image.mime, image.data_base64),
                },
            }));
        }
        Value::Array(parts)
    };
    messages.push(json!({ "role": "user", "content": content }));

    json!({
        "model": settings.model,
        "messages": messages,
        "max_tokens": 1024,
        "stream": stream,
    })
}

/// 组装 Anthropic Messages 协议的请求体；连续同角色轮次会合并。
fn anthropic_body(settings: &Settings, request: &ChatRequest, stream: bool) -> Value {
    let mut messages: Vec<Value> = Vec::with_capacity(request.history.len() + 2);
    for (role, text) in merged_history(request) {
        messages.push(json!({ "role": anthropic_role(role), "content": text }));
    }

    if request.images.is_empty() {
        // 历史末尾若是面试官发言，把当前问题并进同一轮，保持 user/assistant 交替。
        if messages.last().and_then(|message| message["role"].as_str()) == Some("user") {
            if let Some(previous) = messages.last_mut() {
                let merged = match previous["content"].as_str() {
                    Some(prev) => format!("{prev}\n{}", request.user),
                    None => request.user.clone(),
                };
                previous["content"] = Value::String(merged);
            }
        } else {
            messages.push(json!({ "role": "user", "content": request.user }));
        }
    } else {
        let mut parts: Vec<Value> = Vec::with_capacity(request.images.len() + 1);
        // 同上：避免在当前问题前出现连续两条 user 消息。
        if messages.last().and_then(|message| message["role"].as_str()) == Some("user") {
            if let Some(previous) = messages.pop() {
                if let Some(text) = previous["content"].as_str() {
                    parts.push(json!({ "type": "text", "text": text }));
                }
            }
        }
        parts.push(json!({ "type": "text", "text": request.user }));
        for image in &request.images {
            parts.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.mime,
                    "data": image.data_base64,
                },
            }));
        }
        messages.push(json!({ "role": "user", "content": Value::Array(parts) }));
    }

    // Anthropic 要求首条必须是 user；历史以「我」开头时补占位，避免 400。
    ensure_leading_user(
        &mut messages,
        "assistant",
        json!({ "role": "user", "content": LEADING_USER_PLACEHOLDER }),
    );

    json!({
        "model": settings.model,
        "max_tokens": 1024,
        "system": request.system,
        "messages": messages,
        "stream": stream,
    })
}

/// 组装 Gemini generateContent / streamGenerateContent 协议的请求体。
fn gemini_body(request: &ChatRequest) -> Value {
    let mut contents: Vec<Value> = Vec::with_capacity(request.history.len() + 2);
    for (role, text) in merged_history(request) {
        contents.push(json!({
            "role": gemini_role(role),
            "parts": [{ "text": text }],
        }));
    }

    let mut user_parts: Vec<Value> = Vec::with_capacity(request.images.len() + 1);
    user_parts.push(json!({ "text": request.user }));
    for image in &request.images {
        user_parts.push(json!({
            "inline_data": {
                "mime_type": image.mime,
                "data": image.data_base64,
            },
        }));
    }

    // 历史末尾是 user 时并入其 parts，保持 user/model 交替。
    let mut remaining_parts = Some(user_parts);
    if let Some(last) = contents.last_mut() {
        if last["role"].as_str() == Some("user") {
            if let Some(existing) = last["parts"].as_array_mut() {
                if let Some(parts) = remaining_parts.take() {
                    existing.extend(parts);
                }
            }
        }
    }
    if let Some(parts) = remaining_parts {
        contents.push(json!({ "role": "user", "parts": parts }));
    }

    // Gemini 同样要求首条是 user；首条为 model 时补占位。
    ensure_leading_user(
        &mut contents,
        "model",
        json!({ "role": "user", "parts": [{ "text": LEADING_USER_PLACEHOLDER }] }),
    );

    json!({
        "systemInstruction": { "parts": [{ "text": request.system }] },
        "contents": contents,
        "generationConfig": { "maxOutputTokens": 1024 },
    })
}

/// 首字前失败是否值得再试一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryDecision {
    Retry,
    Stop,
}

/// 纯逻辑：仅「未输出任何文本 + 未取消 + 错误可重试 + 还有尝试次数」才重试。
fn retry_decision(
    attempts_made: usize,
    emitted_chars: usize,
    cancelled: bool,
    error: &CoreError,
) -> RetryDecision {
    if attempts_made >= MAX_STREAM_ATTEMPTS {
        return RetryDecision::Stop;
    }
    if cancelled || emitted_chars > 0 || !error.is_retryable() {
        return RetryDecision::Stop;
    }
    RetryDecision::Retry
}

/// 按当前配置发起一次流式对话。
///
/// 每收到一段非空增量就调用一次 `on_delta`；回调返回后检查 `cancel`，
/// 已置位（`true`）则立即停止并返回已生成文本。整体超过 60 秒返回超时错误。
/// 首字前遇到可重试错误会自动重试一次（间隔约 1.5 秒）。
///
/// # 参数
///
/// - `cancel`：跨线程取消标记，由调用方置位；
/// - `on_delta`：增量回调，在异步上下文中同步执行，应保持轻量。
///
/// # 返回
///
/// 返回 [`ChatStats`]，其中 `text` 为完整文本，`ttft_ms` 为首个非空增量的耗时。
pub async fn stream(
    settings: &Settings,
    api_key: &str,
    request: ChatRequest,
    cancel: &std::sync::atomic::AtomicBool,
    mut on_delta: impl FnMut(&str),
) -> Result<ChatStats> {
    let started = Instant::now();
    let client = stream_client().await?;
    let emitted = AtomicUsize::new(0);
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        let outcome = tokio::time::timeout(
            STREAM_TIMEOUT,
            stream_once(
                client,
                settings,
                api_key,
                &request,
                started,
                cancel,
                &mut on_delta,
                &emitted,
            ),
        )
        .await;

        let result = match outcome {
            Ok(result) => result,
            Err(_) => {
                let message = format!("流式响应超时（{} 秒）", STREAM_TIMEOUT.as_secs());
                if emitted.load(Ordering::Relaxed) > 0 {
                    Err(CoreError::Stream(format!(
                        "生成中断：{message}，已输出的内容可能不完整，请点「重新生成」。"
                    )))
                } else {
                    Err(CoreError::Stream(message))
                }
            }
        };

        match result {
            Ok((text, ttft_ms)) => {
                return Ok(ChatStats {
                    text,
                    model: settings.model.clone(),
                    ttft_ms,
                    total_ms: (started.elapsed().as_millis() as u64).max(1),
                });
            }
            Err(error) => {
                let decision = retry_decision(
                    attempts,
                    emitted.load(Ordering::Relaxed),
                    cancel.load(Ordering::Relaxed),
                    &error,
                );
                if decision == RetryDecision::Retry {
                    tracing::warn!(%error, "首字前调用失败，1.5 秒后自动重试一次");
                    tokio::time::sleep(STREAM_RETRY_DELAY).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
}

/// 单次流式尝试，按协议分发。
#[allow(clippy::too_many_arguments)]
async fn stream_once(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
    emitted: &AtomicUsize,
) -> Result<(String, u64)> {
    match settings.provider {
        ProviderKind::OpenAi => {
            openai_stream(
                client, settings, api_key, request, started, cancel, on_delta, emitted,
            )
            .await
        }
        ProviderKind::Anthropic => {
            anthropic_stream(
                client, settings, api_key, request, started, cancel, on_delta, emitted,
            )
            .await
        }
        ProviderKind::Gemini => {
            gemini_stream(
                client, settings, api_key, request, started, cancel, on_delta, emitted,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn openai_stream(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
    emitted: &AtomicUsize,
) -> Result<(String, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Ok((String::new(), 0));
    }

    let url = openai_chat_url(&settings.base_url);
    let body = openai_body(settings, request, true);
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response, settings).await?;

    let mut text = String::new();
    let mut ttft_ms = 0u64;
    pump_sse(
        response,
        |data| {
            if cancel.load(Ordering::Relaxed) {
                return Ok(SseStep::Cancelled);
            }
            let value: Value = match serde_json::from_str(data) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!("跳过无法解析的 OpenAI SSE 数据行：{error}");
                    return Ok(SseStep::Continue);
                }
            };
            if !value["error"].is_null() {
                return Err(CoreError::Api(format!(
                    "OpenAI 流式接口返回错误：{}",
                    truncate(&value["error"].to_string(), 500)
                )));
            }
            let chunk = value["choices"][0]["delta"]["content"]
                .as_str()
                .unwrap_or_default();
            Ok(emit_delta(
                chunk,
                &mut text,
                &mut ttft_ms,
                started,
                cancel,
                on_delta,
                emitted,
            ))
        },
        emitted,
    )
    .await?;

    Ok((text, ttft_ms))
}

#[allow(clippy::too_many_arguments)]
async fn anthropic_stream(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
    emitted: &AtomicUsize,
) -> Result<(String, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Ok((String::new(), 0));
    }

    let url = anthropic_messages_url(&settings.base_url);
    let body = anthropic_body(settings, request, true);
    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response, settings).await?;

    let mut text = String::new();
    let mut ttft_ms = 0u64;
    pump_sse(
        response,
        |data| {
            if cancel.load(Ordering::Relaxed) {
                return Ok(SseStep::Cancelled);
            }
            let value: Value = match serde_json::from_str(data) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!("跳过无法解析的 Anthropic SSE 数据行：{error}");
                    return Ok(SseStep::Continue);
                }
            };
            match value["type"].as_str() {
                Some("content_block_delta") => {
                    let chunk = value["delta"]["text"].as_str().unwrap_or_default();
                    Ok(emit_delta(
                        chunk,
                        &mut text,
                        &mut ttft_ms,
                        started,
                        cancel,
                        on_delta,
                        emitted,
                    ))
                }
                Some("message_stop") => Ok(SseStep::Done),
                Some("error") => Err(CoreError::Api(format!(
                    "Anthropic 流式接口返回错误：{}",
                    truncate(data, 500)
                ))),
                _ => Ok(SseStep::Continue),
            }
        },
        emitted,
    )
    .await?;

    Ok((text, ttft_ms))
}

#[allow(clippy::too_many_arguments)]
async fn gemini_stream(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
    emitted: &AtomicUsize,
) -> Result<(String, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Ok((String::new(), 0));
    }

    let url = gemini_url(&settings.base_url, &settings.model, true);
    let body = gemini_body(request);
    let response = client
        .post(url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response, settings).await?;

    let mut text = String::new();
    let mut ttft_ms = 0u64;
    let mut handle_value = |value: &Value| -> Result<SseStep> {
        if cancel.load(Ordering::Relaxed) {
            return Ok(SseStep::Cancelled);
        }
        if !value["error"].is_null() {
            return Err(CoreError::Api(format!(
                "Gemini 流式接口返回错误：{}",
                truncate(&value["error"].to_string(), 500)
            )));
        }
        let Some(parts) = value["candidates"][0]["content"]["parts"].as_array() else {
            return Ok(SseStep::Continue);
        };
        for part in parts {
            // 2.5 系列会下发 thought 思维链，不能当答案念出来。
            if part["thought"].as_bool() == Some(true) {
                continue;
            }
            let chunk = part["text"].as_str().unwrap_or_default();
            match emit_delta(
                chunk,
                &mut text,
                &mut ttft_ms,
                started,
                cancel,
                on_delta,
                emitted,
            ) {
                SseStep::Continue => {}
                step => return Ok(step),
            }
        }
        Ok(SseStep::Continue)
    };

    // 部分中转/代理忽略 `alt=sse`，直接返回 JSON（单个对象或数组）。
    // 先看 Content-Type，再按首字符兜底，避免把整包 JSON 当 SSE 丢掉。
    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));

    if is_event_stream {
        pump_sse(
            response,
            |data| {
                let value: Value = match serde_json::from_str(data) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::warn!("跳过无法解析的 Gemini SSE 数据行：{error}");
                        return Ok(SseStep::Continue);
                    }
                };
                handle_value(&value)
            },
            emitted,
        )
        .await?;
    } else {
        let raw = response.text().await?;
        let trimmed = raw.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            let value = parse_json_response(trimmed)?;
            let chunks: Vec<&Value> = match &value {
                Value::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            for chunk in chunks {
                if handle_value(chunk)? != SseStep::Continue {
                    break;
                }
            }
        } else {
            // 兜底：按 SSE 逐行处理（某些代理不带 Content-Type）。
            for line in raw.lines() {
                if dispatch_sse_line(line, &mut |data| {
                    let value: Value = match serde_json::from_str(data) {
                        Ok(value) => value,
                        Err(error) => {
                            tracing::warn!("跳过无法解析的 Gemini 数据行：{error}");
                            return Ok(SseStep::Continue);
                        }
                    };
                    handle_value(&value)
                })? != SseStep::Continue
                {
                    break;
                }
            }
        }
    }

    Ok((text, ttft_ms))
}

/// 单个 SSE 事件的处理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SseStep {
    /// 继续读取下一个事件。
    Continue,
    /// 协议层已给出结束标记（如 `[DONE]` / `message_stop`）。
    Done,
    /// 收到取消信号。
    Cancelled,
}

/// 逐行消费 SSE 响应，把每个 `data:` 负载交给 `handle`。
///
/// 空行、注释行与非 `data:` 字段直接跳过；`[DONE]` 统一视为流结束标记
/// （OpenAI 使用，Anthropic / Gemini 不发送）。
///
/// 读取出错时：已输出过文本 → 报「生成中断」（不重试，避免重复内容）；
/// 尚未输出 → 原样向上传递，交给 [`stream`] 判断是否重试。
/// 正常的 EOF 视为结束（部分兼容服务不发 `[DONE]`）。
async fn pump_sse<F>(
    response: reqwest::Response,
    mut handle: F,
    emitted: &AtomicUsize,
) -> Result<()>
where
    F: FnMut(&str) -> Result<SseStep>,
{
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                if emitted.load(Ordering::Relaxed) > 0 {
                    return Err(CoreError::Stream(format!(
                        "生成中断：连接在回答完成前断开（{error}）。已输出的内容可能不完整，请点「重新生成」。"
                    )));
                }
                return Err(CoreError::Http(error));
            }
        };
        buffer.extend_from_slice(&chunk);
        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            let line_bytes: Vec<u8> = buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            if !matches!(dispatch_sse_line(&line, &mut handle)?, SseStep::Continue) {
                return Ok(());
            }
        }
    }

    // 处理末尾没有换行的最后一行。
    if !buffer.is_empty() {
        let line = String::from_utf8_lossy(&buffer);
        let _ = dispatch_sse_line(&line, &mut handle)?;
    }
    Ok(())
}

/// 解析一行并分发；非 `data:` 行返回 `Continue`。
fn dispatch_sse_line<F>(line: &str, handle: &mut F) -> Result<SseStep>
where
    F: FnMut(&str) -> Result<SseStep>,
{
    let Some(data) = parse_sse_data(line) else {
        return Ok(SseStep::Continue);
    };
    if data.trim() == "[DONE]" {
        return Ok(SseStep::Done);
    }
    handle(data)
}

/// 处理一个增量：首个非空增量记录 TTFT，回调后检查取消标记。
#[allow(clippy::too_many_arguments)]
fn emit_delta(
    chunk: &str,
    text: &mut String,
    ttft_ms: &mut u64,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
    emitted: &AtomicUsize,
) -> SseStep {
    if chunk.is_empty() {
        return SseStep::Continue;
    }
    if *ttft_ms == 0 {
        // 至少记 1ms：0 保留给「未收到任何增量」。
        *ttft_ms = (started.elapsed().as_millis() as u64).max(1);
    }
    text.push_str(chunk);
    emitted.fetch_add(chunk.chars().count(), Ordering::Relaxed);
    on_delta(chunk);
    if cancel.load(Ordering::Relaxed) {
        SseStep::Cancelled
    } else {
        SseStep::Continue
    }
}

/// 从一行 SSE 文本中提取 `data:` 字段的负载。
///
/// 返回 `None` 的情况：空行、注释行（以 `:` 开头）、非 `data:` 字段、
/// 以及 `data:` 后没有内容。兼容 CRLF 与 `data:` 后的一个可选空格
/// （多余空格会留给 JSON 解析器，JSON 允许前导空白）。
///
/// 该函数只解析单行，不做多行 data 拼接（本模块用到的协议均为单行 JSON）。
pub fn parse_sse_data(line: &str) -> Option<&str> {
    let line = line.trim_end_matches(|c: char| c == '\r' || c == '\n');
    if line.is_empty() || line.starts_with(':') {
        return None;
    }
    let rest = line.strip_prefix("data:")?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.is_empty() { None } else { Some(rest) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_skips_blank_comment_and_non_data_lines() {
        assert_eq!(parse_sse_data(""), None);
        assert_eq!(parse_sse_data("\r"), None);
        assert_eq!(parse_sse_data("   "), None);
        assert_eq!(parse_sse_data(": keep-alive"), None);
        assert_eq!(parse_sse_data("event: content_block_delta"), None);
        assert_eq!(parse_sse_data("id: 7"), None);
        assert_eq!(parse_sse_data("data:"), None);
        assert_eq!(parse_sse_data("data: "), None);
    }

    #[test]
    fn sse_parser_reads_payload_with_optional_space_and_crlf() {
        assert_eq!(parse_sse_data("data: {\"x\":1}"), Some("{\"x\":1}"));
        assert_eq!(parse_sse_data("data:{\"x\":1}"), Some("{\"x\":1}"));
        assert_eq!(parse_sse_data("data: {\"x\":1}\r\n"), Some("{\"x\":1}"));
        // 多余空格仍保留，JSON 解析器会忽略前导空白。
        assert_eq!(parse_sse_data("data:  {\"x\":1}"), Some(" {\"x\":1}"));
    }

    #[test]
    fn sse_parser_recognizes_done_marker() {
        assert_eq!(parse_sse_data("data: [DONE]"), Some("[DONE]"));
        assert_eq!(parse_sse_data("data:[DONE]"), Some("[DONE]"));
    }

    #[test]
    fn openai_url_handles_all_base_url_shapes() {
        // 裸域名：自动补 /v1。
        assert_eq!(
            openai_chat_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        // 带 /v1（有无尾斜杠）：不重复。
        for base in [
            "https://api.deepseek.com/v1",
            "https://api.deepseek.com/v1/",
        ] {
            assert_eq!(
                openai_chat_url(base),
                "https://api.deepseek.com/v1/chat/completions",
                "base={base}"
            );
        }
        // 用户误填完整端点：原样使用（这正是「测试连接 404」的根因）。
        assert_eq!(
            openai_chat_url("https://tyapi.lxseek.com/v1/chat/completions"),
            "https://tyapi.lxseek.com/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_url("https://gateway.local/chat/completions"),
            "https://gateway.local/chat/completions"
        );
        // 自建网关的非 /v1 版本段保持原样。
        assert_eq!(
            openai_chat_url("https://gateway.local/v2"),
            "https://gateway.local/v2/chat/completions"
        );
        // 路径本身不是版本段时按根处理。
        assert_eq!(
            openai_chat_url("https://gateway.local/api"),
            "https://gateway.local/api/v1/chat/completions"
        );
    }

    #[test]
    fn anthropic_url_does_not_duplicate_v1() {
        for base in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "https://api.anthropic.com/v1",
            "https://api.anthropic.com/v1/",
        ] {
            assert_eq!(
                anthropic_messages_url(base),
                "https://api.anthropic.com/v1/messages",
                "base={base}"
            );
        }
        // 用户误填完整端点时原样使用。
        assert_eq!(
            anthropic_messages_url("https://proxy.local/v1/messages"),
            "https://proxy.local/v1/messages"
        );
    }

    #[test]
    fn gemini_url_does_not_duplicate_v1beta_or_models_prefix() {
        assert_eq!(
            gemini_url(
                "https://generativelanguage.googleapis.com",
                "gemini-2.5-flash",
                true
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            gemini_url(
                "https://generativelanguage.googleapis.com/v1beta/",
                "models/gemini-2.5-flash",
                false
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
    }

    fn sample_request() -> ChatRequest {
        ChatRequest {
            system: "系统提示".into(),
            user: "当前问题".into(),
            history: vec![
                Turn {
                    role: TurnRole::Interviewer,
                    text: "上一问".into(),
                },
                Turn {
                    role: TurnRole::Interviewer,
                    text: "补充".into(),
                },
                Turn {
                    role: TurnRole::Me,
                    text: "上一答".into(),
                },
            ],
            images: vec![ImagePart {
                mime: "image/jpeg".into(),
                data_base64: "QUJD".into(),
            }],
        }
    }

    #[test]
    fn openai_body_maps_history_and_images() {
        let settings = Settings::default();
        let body = openai_body(&settings, &sample_request(), true);

        assert_eq!(body["stream"], true);
        let messages = body["messages"].as_array().expect("messages 应为数组");
        // system + 合并后的面试官发言 + 我的上一答 + 当前问题
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "上一问\n补充");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["content"][0]["text"], "当前问题");
        assert_eq!(
            messages[3]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,QUJD"
        );
    }

    #[test]
    fn anthropic_body_merges_consecutive_roles_and_images() {
        let settings = Settings::default();
        let body = anthropic_body(&settings, &sample_request(), true);

        assert_eq!(body["stream"], true);
        let messages = body["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["content"], "上一问\n补充");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["content"][1]["type"], "image");
        assert_eq!(
            messages[2]["content"][1]["source"]["media_type"],
            "image/jpeg"
        );
        assert_eq!(messages[2]["content"][1]["source"]["data"], "QUJD");
    }

    #[test]
    fn anthropic_body_merges_final_user_into_previous_user_turn() {
        let request = ChatRequest {
            system: "s".into(),
            user: "当前问题".into(),
            history: vec![Turn {
                role: TurnRole::Interviewer,
                text: "上一问".into(),
            }],
            images: Vec::new(),
        };
        let body = anthropic_body(&Settings::default(), &request, false);
        let messages = body["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "上一问\n当前问题");
    }

    #[test]
    fn anthropic_body_inserts_leading_user_when_history_starts_with_me() {
        let request = ChatRequest {
            system: "s".into(),
            user: "当前问题".into(),
            history: vec![
                Turn {
                    role: TurnRole::Me,
                    text: "我先把背景讲完".into(),
                },
                Turn {
                    role: TurnRole::Interviewer,
                    text: "上一问".into(),
                },
            ],
            images: Vec::new(),
        };
        let body = anthropic_body(&Settings::default(), &request, false);
        let messages = body["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], LEADING_USER_PLACEHOLDER);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
        // 首尾都必须是 user。
        assert_eq!(messages.last().unwrap()["role"], "user");
    }

    #[test]
    fn gemini_body_maps_roles_and_inline_data() {
        let body = gemini_body(&sample_request());

        let contents = body["contents"].as_array().expect("contents 应为数组");
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "上一问\n补充");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[2]["parts"][0]["text"], "当前问题");
        assert_eq!(
            contents[2]["parts"][1]["inline_data"]["mime_type"],
            "image/jpeg"
        );
        assert_eq!(contents[2]["parts"][1]["inline_data"]["data"], "QUJD");
    }

    #[test]
    fn gemini_body_inserts_leading_user_when_history_starts_with_model() {
        let request = ChatRequest {
            system: "s".into(),
            user: "当前问题".into(),
            history: vec![
                Turn {
                    role: TurnRole::Me,
                    text: "我先讲".into(),
                },
                Turn {
                    role: TurnRole::Interviewer,
                    text: "上一问".into(),
                },
            ],
            images: Vec::new(),
        };
        let body = gemini_body(&request);
        let contents = body["contents"].as_array().expect("contents 应为数组");
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], LEADING_USER_PLACEHOLDER);
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents.last().unwrap()["role"], "user");
    }

    #[test]
    fn gemini_text_of_skips_thought_parts() {
        let value = json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "正在分析…", "thought": true },
                    { "text": "先给结论。" }
                ]}
            }]
        });
        assert_eq!(gemini_text_of(&value).as_deref(), Some("先给结论。"));
        let thought_only = json!({
            "candidates": [{ "content": { "parts": [{ "text": "x", "thought": true }] } }]
        });
        assert_eq!(gemini_text_of(&thought_only), None);
    }

    #[test]
    fn retry_only_before_first_token_and_for_retryable_errors() {
        let retryable = CoreError::Stream("超时".into());
        assert_eq!(
            retry_decision(1, 0, false, &retryable),
            RetryDecision::Retry
        );
        // 已经输出过文本：不重试，避免重复内容。
        assert_eq!(retry_decision(1, 1, false, &retryable), RetryDecision::Stop);
        // 已取消：不重试。
        assert_eq!(retry_decision(1, 0, true, &retryable), RetryDecision::Stop);
        // 已经用完尝试次数：不重试。
        assert_eq!(retry_decision(2, 0, false, &retryable), RetryDecision::Stop);
        // 不可重试的错误（如 401）：不重试。
        let unauthorized = CoreError::HttpStatus {
            status: 401,
            advice: "Key 无效".into(),
            body: "".into(),
        };
        assert_eq!(
            retry_decision(1, 0, false, &unauthorized),
            RetryDecision::Stop
        );
    }

    #[test]
    fn status_advice_is_chinese_and_actionable() {
        let settings = Settings {
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            ..Settings::default()
        };
        assert!(status_advice(401, &settings).contains("API Key 无效"));
        assert!(status_advice(403, &settings).contains("权限"));
        let not_found = status_advice(404, &settings);
        assert!(not_found.contains("https://api.deepseek.com/v1"));
        assert!(not_found.contains("deepseek-chat"));
        assert!(status_advice(429, &settings).contains("重新生成"));
        assert!(status_advice(503, &settings).contains("服务端异常"));
    }

    #[test]
    fn http_error_status_is_not_retryable_for_auth_but_is_for_429() {
        let settings = Settings::default();
        let _ = settings;
        let unauthorized = CoreError::HttpStatus {
            status: 401,
            advice: "x".into(),
            body: "y".into(),
        };
        assert!(!unauthorized.is_retryable());
        let limited = CoreError::HttpStatus {
            status: 429,
            advice: "x".into(),
            body: "y".into(),
        };
        assert!(limited.is_retryable());
    }
}
