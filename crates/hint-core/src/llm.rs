//! 大模型调用：三协议非流式 [`complete`] 与流式 [`stream`]。
//!
//! - OpenAI 兼容（DeepSeek / 通义 / 智谱 / Ollama / vLLM / one-api 等）：
//!   `POST {base}/chat/completions`，SSE 取 `choices[0].delta.content`；
//! - Anthropic Messages：`POST {base}/v1/messages`，SSE 取 `content_block_delta.delta.text`；
//! - Gemini：`POST {base}/v1beta/models/{model}:streamGenerateContent?alt=sse`，
//!   SSE 取 `candidates[0].content.parts[].text`。
//!
//! 历史消息统一按「面试官 = user 侧，我 = assistant/model 侧」映射。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{CoreError, Result};
use crate::settings::{ProviderKind, Settings};

/// 流式调用的总超时（从发起请求到收到最后一个增量）。
const STREAM_TIMEOUT: Duration = Duration::from_secs(60);

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

/// 非 2xx 响应统一转成带状态码与响应体前 500 字符的错误。
async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let raw = match response.text().await {
        Ok(raw) => raw,
        Err(_) => String::new(),
    };
    Err(CoreError::Api(format!(
        "HTTP {status}: {}",
        truncate(&raw, 500)
    )))
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
    let url = format!("{}/chat/completions", base(&settings.base_url));
    let body = openai_body(settings, &request, false);
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response).await?;
    let raw = response.text().await?;

    let value: serde_json::Value = serde_json::from_str(&raw)?;
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
    let url = format!("{}/v1/messages", base(&settings.base_url));
    let body = anthropic_body(settings, &request, false);
    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response).await?;
    let raw = response.text().await?;

    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let text = ensure_text(value["content"][0]["text"].as_str(), &raw)?;
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
    let url = format!(
        "{}/v1beta/models/{}:generateContent",
        base(&settings.base_url),
        settings.model
    );
    let body = gemini_body(&request);
    let response = client
        .post(url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response).await?;
    let raw = response.text().await?;

    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let text = ensure_text(
        value["candidates"][0]["content"]["parts"][0]["text"].as_str(),
        &raw,
    )?;
    Ok(ChatResponse {
        text,
        model: settings.model.clone(),
    })
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

/// Gemini 协议中的角色名。
fn gemini_role(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Interviewer => "user",
        TurnRole::Me => "model",
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
    let mut messages: Vec<Value> = Vec::with_capacity(request.history.len() + 1);
    for (role, text) in merged_history(request) {
        let role = match role {
            TurnRole::Interviewer => "user",
            TurnRole::Me => "assistant",
        };
        messages.push(json!({ "role": role, "content": text }));
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
    let mut contents: Vec<Value> = Vec::with_capacity(request.history.len() + 1);
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

    json!({
        "systemInstruction": { "parts": [{ "text": request.system }] },
        "contents": contents,
        "generationConfig": { "maxOutputTokens": 1024 },
    })
}

/// 按当前配置发起一次流式对话。
///
/// 每收到一段非空增量就调用一次 `on_delta`；回调返回后检查 `cancel`，
/// 已置位（`true`）则立即停止并返回已生成文本。整体超过 60 秒返回超时错误。
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

    // 三个分支返回同一类型：Result<Result<(String, u64), CoreError>, Elapsed>。
    let outcome = match settings.provider {
        ProviderKind::OpenAi => {
            tokio::time::timeout(
                STREAM_TIMEOUT,
                openai_stream(
                    &client,
                    settings,
                    api_key,
                    &request,
                    started,
                    cancel,
                    &mut on_delta,
                ),
            )
            .await
        }
        ProviderKind::Anthropic => {
            tokio::time::timeout(
                STREAM_TIMEOUT,
                anthropic_stream(
                    &client,
                    settings,
                    api_key,
                    &request,
                    started,
                    cancel,
                    &mut on_delta,
                ),
            )
            .await
        }
        ProviderKind::Gemini => {
            tokio::time::timeout(
                STREAM_TIMEOUT,
                gemini_stream(
                    &client,
                    settings,
                    api_key,
                    &request,
                    started,
                    cancel,
                    &mut on_delta,
                ),
            )
            .await
        }
    };

    let (text, ttft_ms) = match outcome {
        Ok(result) => result?,
        Err(_) => {
            return Err(CoreError::Api(format!(
                "流式响应超时（{} 秒）",
                STREAM_TIMEOUT.as_secs()
            )));
        }
    };

    Ok(ChatStats {
        text,
        model: settings.model.clone(),
        ttft_ms,
        total_ms: (started.elapsed().as_millis() as u64).max(1),
    })
}

async fn openai_stream(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Result<(String, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Ok((String::new(), 0));
    }

    let url = format!("{}/chat/completions", base(&settings.base_url));
    let body = openai_body(settings, request, true);
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response).await?;

    let mut text = String::new();
    let mut ttft_ms = 0u64;
    pump_sse(response, |data| {
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
        ))
    })
    .await?;

    Ok((text, ttft_ms))
}

async fn anthropic_stream(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Result<(String, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Ok((String::new(), 0));
    }

    let url = format!("{}/v1/messages", base(&settings.base_url));
    let body = anthropic_body(settings, request, true);
    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response).await?;

    let mut text = String::new();
    let mut ttft_ms = 0u64;
    pump_sse(response, |data| {
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
                ))
            }
            Some("message_stop") => Ok(SseStep::Done),
            Some("error") => Err(CoreError::Api(format!(
                "Anthropic 流式接口返回错误：{}",
                truncate(data, 500)
            ))),
            _ => Ok(SseStep::Continue),
        }
    })
    .await?;

    Ok((text, ttft_ms))
}

async fn gemini_stream(
    client: &reqwest::Client,
    settings: &Settings,
    api_key: &str,
    request: &ChatRequest,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Result<(String, u64)> {
    if cancel.load(Ordering::Relaxed) {
        return Ok((String::new(), 0));
    }

    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
        base(&settings.base_url),
        settings.model
    );
    let body = gemini_body(request);
    let response = client
        .post(url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?;
    let response = ensure_success(response).await?;

    let mut text = String::new();
    let mut ttft_ms = 0u64;
    pump_sse(response, |data| {
        if cancel.load(Ordering::Relaxed) {
            return Ok(SseStep::Cancelled);
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("跳过无法解析的 Gemini SSE 数据行：{error}");
                return Ok(SseStep::Continue);
            }
        };
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
            let chunk = part["text"].as_str().unwrap_or_default();
            match emit_delta(chunk, &mut text, &mut ttft_ms, started, cancel, on_delta) {
                SseStep::Continue => {}
                step => return Ok(step),
            }
        }
        Ok(SseStep::Continue)
    })
    .await?;

    Ok((text, ttft_ms))
}

/// 单个 SSE 事件的处理结果。
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
/// （OpenAI 使用，Anthropic / Gemini 不发送）。读取错误向上传播，
/// 解析失败的数据行由 `handle` 负责记录并跳过。
async fn pump_sse<F>(response: reqwest::Response, mut handle: F) -> Result<()>
where
    F: FnMut(&str) -> Result<SseStep>,
{
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
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
fn emit_delta(
    chunk: &str,
    text: &mut String,
    ttft_ms: &mut u64,
    started: Instant,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> SseStep {
    if chunk.is_empty() {
        return SseStep::Continue;
    }
    if *ttft_ms == 0 {
        // 至少记 1ms：0 保留给「未收到任何增量」。
        *ttft_ms = (started.elapsed().as_millis() as u64).max(1);
    }
    text.push_str(chunk);
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
/// 以及 `data:` 后没有内容。兼容 CRLF 与 `data:` 后的一个可选空格。
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
    }

    #[test]
    fn sse_parser_recognizes_done_marker() {
        assert_eq!(parse_sse_data("data: [DONE]"), Some("[DONE]"));
        assert_eq!(parse_sse_data("data:[DONE]"), Some("[DONE]"));
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
}
