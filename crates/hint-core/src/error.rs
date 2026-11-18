use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("凭证读写失败：{0}")]
    Keyring(#[from] keyring::Error),

    #[error("{}", transport_message(.0))]
    Http(#[from] reqwest::Error),

    #[error("接口返回错误：{0}")]
    Api(String),

    /// 模型接口返回非 2xx：`advice` 是给用户看的中文建议，`body` 是原始响应片段。
    #[error("【HTTP {status}】{advice}\n原始响应：{body}")]
    HttpStatus {
        status: u16,
        advice: String,
        body: String,
    },

    /// 流式响应超时或中断（`message` 已包含中文说明）。
    ///
    /// 该错误本身属于「可重试」类别；是否真的重试由调用方根据
    /// 「是否已经向用户输出过文本」决定。
    #[error("{0}")]
    Stream(String),

    #[error("文件读写失败：{0}")]
    Io(#[from] std::io::Error),

    #[error("配置解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

impl CoreError {
    /// 该错误是否值得原样重试（网络抖动、服务端临时异常、限流）。
    ///
    /// 不可重试：401/403/404、参数/序列化错误、非法配置。
    /// 对 [`CoreError::Stream`]，调用方还需确认尚未输出任何文本后再重试。
    pub fn is_retryable(&self) -> bool {
        match self {
            CoreError::Http(error) => {
                error.is_timeout() || error.is_connect() || error.is_body() || error.is_decode()
            }
            CoreError::HttpStatus { status, .. } => *status == 429 || *status >= 500,
            CoreError::Stream(_) => true,
            _ => false,
        }
    }
}

/// 把 reqwest 的传输层错误翻成用户能读懂的中文；保留原始错误便于排查。
fn transport_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        format!("模型接口响应超时（{error}）。请检查网络或代理后重试。")
    } else if error.is_connect() {
        format!("无法连接模型接口（{error}）。请检查网络或代理后重试。")
    } else if error.is_body() || error.is_decode() {
        format!("读取模型响应时连接中断（{error}）。请检查网络后点「重新生成」。")
    } else if error.is_builder() {
        format!("模型接口地址无效（{error}）。请在设置页检查 Base URL。")
    } else {
        format!("模型接口请求失败（{error}）。请检查网络或代理后重试。")
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn status_error(status: u16) -> CoreError {
        CoreError::HttpStatus {
            status,
            advice: "建议".into(),
            body: "body".into(),
        }
    }

    #[test]
    fn retryable_status_codes_are_429_and_5xx() {
        assert!(status_error(429).is_retryable());
        assert!(status_error(500).is_retryable());
        assert!(status_error(503).is_retryable());
    }

    #[test]
    fn auth_and_not_found_errors_are_not_retryable() {
        for status in [400, 401, 403, 404, 422] {
            assert!(
                !status_error(status).is_retryable(),
                "HTTP {status} 不应重试"
            );
        }
    }

    #[test]
    fn stream_errors_are_retryable_but_api_errors_are_not() {
        assert!(CoreError::Stream("流式响应超时".into()).is_retryable());
        assert!(!CoreError::Api("响应中没有可用文本".into()).is_retryable());
        let json = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        assert!(!CoreError::Json(json).is_retryable());
    }

    #[test]
    fn http_status_display_keeps_chinese_advice_and_raw_body() {
        let message = status_error(401).to_string();
        assert!(message.contains("【HTTP 401】"));
        assert!(message.contains("建议"));
        assert!(message.contains("原始响应：body"));
    }
}
