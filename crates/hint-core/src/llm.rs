use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::settings::{ProviderKind, Settings};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub text: String,
    pub model: String,
}

fn base(base_url: &str) -> &str {
    base_url.trim().trim_end_matches('/')
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?)
}

fn ensure_text(text: Option<&str>, raw: &str) -> Result<String> {
    match text {
        Some(text) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        _ => Err(CoreError::Api(format!("响应中没有可用文本：{raw}"))),
    }
}

/// 按当前配置调用一次非流式对话。M1 会在此基础上换成流式实现。
pub async fn complete(settings: &Settings, api_key: &str, request: ChatRequest) -> Result<ChatResponse> {
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
    let body = serde_json::json!({
        "model": settings.model,
        "messages": [
            { "role": "system", "content": request.system },
            { "role": "user", "content": request.user }
        ],
        "max_tokens": 1024,
        "stream": false
    });

    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let raw = response.text().await?;
    if !status.is_success() {
        return Err(CoreError::Api(format!("HTTP {status}: {raw}")));
    }

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
    let body = serde_json::json!({
        "model": settings.model,
        "max_tokens": 1024,
        "system": request.system,
        "messages": [
            { "role": "user", "content": request.user }
        ]
    });

    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let raw = response.text().await?;
    if !status.is_success() {
        return Err(CoreError::Api(format!("HTTP {status}: {raw}")));
    }

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
    let body = serde_json::json!({
        "systemInstruction": {
            "parts": [{ "text": request.system }]
        },
        "contents": [
            {
                "role": "user",
                "parts": [{ "text": request.user }]
            }
        ],
        "generationConfig": { "maxOutputTokens": 1024 }
    });

    let response = client
        .post(url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let raw = response.text().await?;
    if !status.is_success() {
        return Err(CoreError::Api(format!("HTTP {status}: {raw}")));
    }

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
