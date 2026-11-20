//! 统一错误类型与下载失败分类。
//!
//! 错误消息进入日志或 UI 之前必须先脱敏：任何 URL 只保留 origin + path，
//! query 一律丢弃（HF 的 Xet 签名 URL 带临时凭证）。

use thiserror::Error;

/// 下载/安装失败的稳定分类。UI 按它映射文案，下载器按它决定重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureReason {
    /// 连接中断、连接被重置等可重试的网络问题。
    Network,
    /// DNS 解析失败。
    Dns,
    /// 超时（连接、读、无数据判死）。
    Timeout,
    /// TLS 证书校验失败（代理劫持、系统时间错误）。
    Certificate,
    /// 服务器返回了非 2xx 的 HTTP 状态。
    Http,
    /// 字节数或 sha256 与清单不符。
    Integrity,
    /// 磁盘空间不足或无法写入。
    Storage,
    /// 平台不支持本地识别。
    Unsupported,
    /// 其它未归类的错误。
    Unknown,
}

impl FailureReason {
    /// 同一源上值得重试（结果可能不同）的分类。
    pub fn is_transient(self) -> bool {
        matches!(self, FailureReason::Network | FailureReason::Timeout)
    }

    /// 换源也无济于事的分类：本地磁盘问题、清单/程序错误、校验失败。
    ///
    /// `Integrity` 按清单文档 §5 视为终态：239MB 模型换了源也多半是同一个坏文件，
    /// 立即失败交给用户重试，比默默换源更可诊断。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            FailureReason::Storage
                | FailureReason::Integrity
                | FailureReason::Unsupported
                | FailureReason::Unknown
        )
    }

    /// 面向用户的一句话提示。
    pub fn user_message(self) -> &'static str {
        match self {
            FailureReason::Network => "网络连接中断。已下载部分会保留，恢复网络后继续。",
            FailureReason::Dns => "无法解析下载地址。请检查网络或 DNS 设置。",
            FailureReason::Timeout => "下载超时。已下载部分会保留，请重试。",
            FailureReason::Certificate => "证书校验失败。可能存在代理劫持或系统时间错误。",
            FailureReason::Http => "服务器返回错误。请稍后重试。",
            FailureReason::Integrity => "文件校验失败，已删除损坏部分。请重新下载。",
            // 具体需要多少空间随续传进度变化（首次约 300MB，续传更少），
            // 由错误消息本身给出，这里不再写死数字，避免误导。
            FailureReason::Storage => "无法写入模型文件。请检查磁盘剩余空间与目录权限。",
            FailureReason::Unsupported => "当前系统不支持本地语音识别。",
            FailureReason::Unknown => "发生未知错误，请重试。",
        }
    }
}

/// `hint-stt` 的全部错误。
#[derive(Debug, Error)]
pub enum SttError {
    /// 网络层失败（连接、DNS、超时、证书、HTTP 状态）。
    #[error("network error: {message}")]
    Network {
        reason: FailureReason,
        /// 已脱敏的消息，可直接进日志。
        message: String,
        /// HTTP 状态码（若有）。
        status: Option<u16>,
    },

    /// 校验失败：字节数或 sha256 不符。
    #[error("integrity check failed: {message}")]
    Integrity { message: String },

    /// 本地存储失败：创建目录、写入、rename、磁盘不足。
    #[error("storage error: {message}")]
    Storage { message: String },

    /// 当前平台不支持本地语音识别。
    #[error("unsupported platform: {message}")]
    Unsupported { message: String },

    /// 请求/响应格式或状态不合法（HTTP 400/413，WAV 格式错误等）。
    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    /// 配置缺失或不合法（worker 启动、WorkerConfig 校验）。
    #[error("invalid configuration: {message}")]
    Config { message: String },

    /// sidecar 进程层面的失败：启动超时、异常退出、端口行非法。
    #[error("worker process error: {message}")]
    Worker { message: String },

    /// 推理引擎初始化或推理失败。
    #[error("engine error: {message}")]
    Engine { message: String },

    /// 序列化/反序列化失败。
    #[error("json error: {message}")]
    Json { message: String },
}

/// 用户主动取消下载时使用的固定消息，便于跨层识别「取消」而不是「失败」。
pub const CANCELLED_MESSAGE: &str = "download cancelled";

/// Worker 进程层的英文运行时错误 → 用户可读的中文。
///
/// 只翻译已知的几种；其余（尤其是 `diagnose_startup_failure` 生成的中文诊断）
/// 原样返回。
fn worker_message(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("inference aborted by the watchdog") {
        return "语音识别卡住，已中止当前任务并重启引擎。".to_string();
    }
    if lower.contains("did not answer within") {
        return "语音引擎长时间无响应，已重启。".to_string();
    }
    if lower.contains("cannot connect to worker") {
        return "无法连接本地语音引擎，正在重新启动。".to_string();
    }
    if lower.contains("did not report its port within") {
        return "语音引擎启动超时，请重试。".to_string();
    }
    if lower.contains("exited during startup") {
        return "语音引擎启动失败，请检查模型文件后重试。".to_string();
    }
    if lower.contains("closed stdout before reporting its port") {
        return "语音引擎意外退出，请重试。".to_string();
    }
    message.to_string()
}

impl SttError {
    /// 构造网络错误并自动脱敏消息。
    pub fn network(reason: FailureReason, message: impl Into<String>, status: Option<u16>) -> Self {
        SttError::Network {
            reason,
            message: scrub_urls(&message.into()),
            status,
        }
    }

    /// 构造存储错误。
    pub fn storage(message: impl Into<String>) -> Self {
        SttError::Storage {
            message: message.into(),
        }
    }

    /// 稳定的失败分类；非下载类错误映射为 `Unknown`。
    pub fn reason(&self) -> FailureReason {
        match self {
            SttError::Network { reason, .. } => *reason,
            SttError::Integrity { .. } => FailureReason::Integrity,
            SttError::Storage { .. } => FailureReason::Storage,
            SttError::Unsupported { .. } => FailureReason::Unsupported,
            _ => FailureReason::Unknown,
        }
    }

    /// 已脱敏的展示消息。
    pub fn user_message(&self) -> String {
        match self {
            SttError::Network { message, .. } => {
                format!("{} {}", self.reason().user_message(), message)
            }
            // 磁盘不足最常见的来源是写入时 OS 报错，底层英文不易读，统一补一句中文指引。
            SttError::Storage { message } => {
                format!("{}（{message}）", FailureReason::Storage.user_message())
            }
            SttError::Integrity { message } => {
                format!("{}（{message}）", FailureReason::Integrity.user_message())
            }
            // Engine / Worker 的 message 多数已是中文诊断（见 diagnose_startup_failure），
            // 直接展示；Worker 里少数英文运行时错误先翻译成中文。
            SttError::Engine { message } => message.clone(),
            SttError::Worker { message } => worker_message(message),
            SttError::Config { message } => {
                if message == CANCELLED_MESSAGE {
                    "下载已取消".to_string()
                } else {
                    format!("配置错误：{message}")
                }
            }
            other => other.to_string(),
        }
    }

    /// 是否为用户主动取消（不是失败）。
    pub fn is_cancelled(&self) -> bool {
        matches!(self, SttError::Config { message } if message == CANCELLED_MESSAGE)
    }
}

impl From<std::io::Error> for SttError {
    fn from(error: std::io::Error) -> Self {
        SttError::storage(error.to_string())
    }
}

impl From<serde_json::Error> for SttError {
    fn from(error: serde_json::Error) -> Self {
        SttError::Json {
            message: error.to_string(),
        }
    }
}

/// 结果别名。
pub type Result<T> = std::result::Result<T, SttError>;

// ---------------------------------------------------------------------------
// URL 脱敏
// ---------------------------------------------------------------------------

/// 把一个绝对 URL 收敛成 `origin + path`：去掉 userinfo、query、fragment。
///
/// 解析失败时返回 `"url"`，绝不回显原文。
pub fn sanitize_url(url: &str) -> String {
    match UrlParts::parse(url) {
        Some(parts) => format!("{}://{}{}", parts.scheme, parts.authority, parts.path),
        None => "url".to_string(),
    }
}

/// 只保留 origin（`scheme://host[:port]`），连 path 也丢掉。
pub fn sanitize_origin(url: &str) -> String {
    match UrlParts::parse(url) {
        Some(parts) => format!("{}://{}", parts.scheme, parts.authority),
        None => "url".to_string(),
    }
}

/// 把消息里出现的所有 `http(s)://...` 片段替换成 origin + path。
pub fn scrub_urls(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = find_scheme(rest) {
        let (prefix, tail) = rest.split_at(start);
        out.push_str(prefix);
        // URL 片段在空白、引号、尖括号、括号处结束。
        let end = tail
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '}')
            })
            .unwrap_or(tail.len());
        let raw = &tail[..end];
        out.push_str(&sanitize_url(raw));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn find_scheme(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let http = lower.find("http://");
    let https = lower.find("https://");
    match (http, https) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

struct UrlParts {
    scheme: String,
    authority: String,
    path: String,
}

impl UrlParts {
    /// 手写的最小 URL 解析，避免为此引入 `url` 依赖。
    fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
            return None;
        }
        // authority 到第一个 `/`、`?`、`#` 为止。
        let authority_end = rest
            .find(|c: char| matches!(c, '/' | '?' | '#'))
            .unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        if authority.is_empty() {
            return None;
        }
        // 去掉 userinfo。
        let host = match authority.rsplit_once('@') {
            Some((_, host)) => host,
            None => authority,
        };
        if host.is_empty() {
            return None;
        }
        let after = &rest[authority_end..];
        let path_end = after
            .find(|c: char| matches!(c, '?' | '#'))
            .unwrap_or(after.len());
        let path = &after[..path_end];
        Some(UrlParts {
            scheme: scheme.to_ascii_lowercase(),
            authority: host.to_string(),
            path: if path.is_empty() {
                "/".to_string()
            } else {
                path.to_string()
            },
        })
    }
}

// ---------------------------------------------------------------------------
// 底层错误归类
// ---------------------------------------------------------------------------

/// 把任意底层错误映射成 [`FailureReason`]。
pub fn classify_error(error: &SttError) -> FailureReason {
    match error {
        SttError::Network { reason, .. } => *reason,
        SttError::Integrity { .. } => FailureReason::Integrity,
        SttError::Storage { .. } => FailureReason::Storage,
        SttError::Unsupported { .. } => FailureReason::Unsupported,
        _ => FailureReason::Unknown,
    }
}

/// 归类 `std::io::Error`：磁盘相关 vs 网络相关。
pub fn classify_io_error(error: &std::io::Error) -> FailureReason {
    use std::io::ErrorKind as K;
    match error.kind() {
        K::StorageFull | K::PermissionDenied | K::ReadOnlyFilesystem | K::FileTooLarge => {
            FailureReason::Storage
        }
        K::TimedOut | K::WouldBlock => FailureReason::Timeout,
        K::ConnectionAborted
        | K::ConnectionRefused
        | K::ConnectionReset
        | K::BrokenPipe
        | K::NotConnected
        | K::UnexpectedEof => FailureReason::Network,
        _ => classify_io_message(&error.to_string()),
    }
}

/// 归类底层库的文本（ureq 把 DNS/证书错误压成字符串）。
pub fn classify_io_message(text: &str) -> FailureReason {
    let lower = text.to_ascii_lowercase();
    if lower.contains("dns")
        || lower.contains("name resolution")
        || lower.contains("getaddrinfo")
        || lower.contains("failed to lookup address")
    {
        return FailureReason::Dns;
    }
    if lower.contains("certificate")
        || lower.contains("cert_")
        || lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("unknownissuer")
        || lower.contains("self-signed")
        || lower.contains("self signed")
    {
        return FailureReason::Certificate;
    }
    if lower.contains("timeout") || lower.contains("timed out") || lower.contains("time out") {
        return FailureReason::Timeout;
    }
    if lower.contains("connection")
        || lower.contains("network")
        || lower.contains("unreachable")
        || lower.contains("reset")
        || lower.contains("closed")
        || lower.contains("broken pipe")
        || lower.contains("unexpected eof")
    {
        return FailureReason::Network;
    }
    FailureReason::Unknown
}

/// 归类 `ureq` 传输错误。
pub fn classify_transport(transport: &ureq::Transport) -> FailureReason {
    use ureq::ErrorKind as K;
    match transport.kind() {
        K::Dns => FailureReason::Dns,
        K::ConnectionFailed
        | K::TooManyRedirects
        | K::Io
        | K::ProxyConnect
        | K::ProxyUnauthorized => {
            // Io 的底层原因可能是 TLS、DNS 或普通网络中断，看文本再细分。
            match transport.message() {
                Some(message) => match classify_io_message(message) {
                    FailureReason::Unknown => FailureReason::Network,
                    other => other,
                },
                None => FailureReason::Network,
            }
        }
        K::InvalidUrl | K::UnknownScheme => FailureReason::Unknown,
        K::BadStatus | K::BadHeader | K::HTTP => FailureReason::Http,
        K::InsecureRequestHttpsOnly | K::InvalidProxyUrl => FailureReason::Unknown,
    }
}

/// 归类 `ureq` 错误。
pub fn classify_ureq_error(error: &ureq::Error) -> FailureReason {
    match error {
        ureq::Error::Status(code, _) => match *code {
            408 | 504 => FailureReason::Timeout,
            _ => FailureReason::Http,
        },
        ureq::Error::Transport(transport) => classify_transport(transport),
    }
}

/// 把 `ureq` 错误转成带分类的 [`SttError`]，消息自动脱敏。
pub fn from_ureq(error: ureq::Error, resource: &str) -> SttError {
    match error {
        ureq::Error::Status(code, response) => {
            // 丢弃 body：里面可能带签名 URL，不读进日志。
            let _ = response;
            // 408/429/5xx 是临时性状态：归类为可重试的 Network，让下载器
            // 在同一源上按退避重试（模块文档承诺「每源重试 5 次」）。
            // 其余 4xx 属于配置/协议问题，换源也无用，保持 Http。
            let reason = match code {
                408 | 429 | 500..=599 => FailureReason::Network,
                _ => FailureReason::Http,
            };
            SttError::network(
                reason,
                format!("HTTP {code} while fetching {resource}"),
                Some(code),
            )
        }
        ureq::Error::Transport(transport) => {
            let reason = classify_transport(&transport);
            let message = transport
                .message()
                .map(|m| format!("{m} while fetching {resource}"))
                .unwrap_or_else(|| format!("transport error while fetching {resource}"));
            SttError::network(reason, message, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_urls_removes_query_and_userinfo() {
        let raw = "GET https://us.aws.cdn.hf.co/xet-bridge-us/abc/model.onnx?X-Amz-Signature=deadbeef&X-Amz-Credential=key failed";
        let cleaned = scrub_urls(raw);
        assert!(!cleaned.contains("deadbeef"));
        assert!(!cleaned.contains("X-Amz"));
        assert!(cleaned.contains("https://us.aws.cdn.hf.co/xet-bridge-us/abc/model.onnx"));
        assert!(cleaned.ends_with(" failed"));
    }

    #[test]
    fn scrub_urls_handles_userinfo_and_multiple_urls() {
        let raw = "a https://user:pass@example.com:8443/p?q=1 b https://huggingface.co/repo/resolve/rev/f.bin#frag c";
        let cleaned = scrub_urls(raw);
        assert!(!cleaned.contains("pass"));
        assert!(!cleaned.contains("q=1"));
        assert!(!cleaned.contains("frag"));
        assert!(cleaned.contains("https://example.com:8443/p"));
        assert!(cleaned.contains("https://huggingface.co/repo/resolve/rev/f.bin"));
    }

    #[test]
    fn scrub_urls_leaves_plain_text_alone() {
        assert_eq!(scrub_urls("plain text"), "plain text");
        assert_eq!(sanitize_url("not a url"), "url");
    }

    #[test]
    fn classification_covers_the_required_categories() {
        assert_eq!(
            classify_io_message("failed to lookup address information: nodename nor servname"),
            FailureReason::Dns
        );
        assert_eq!(
            classify_io_message("certificate verify failed: UnknownIssuer"),
            FailureReason::Certificate
        );
        assert_eq!(
            classify_io_message("operation timed out"),
            FailureReason::Timeout
        );
        assert_eq!(
            classify_io_message("connection reset by peer"),
            FailureReason::Network
        );
        assert_eq!(classify_io_message("something odd"), FailureReason::Unknown);
        assert_eq!(
            classify_io_error(&std::io::Error::from(std::io::ErrorKind::StorageFull)),
            FailureReason::Storage
        );
    }

    #[test]
    fn storage_message_is_prefixed_with_chinese_guidance() {
        let error = SttError::storage("cannot write x.part: 磁盘空间不足");
        let message = error.user_message();
        assert!(message.contains("无法写入模型文件"), "{message}");
        assert!(message.contains("磁盘剩余空间"), "{message}");
        // 具体需要多少空间由调用方按实际预算写进消息，这里不应写死数字。
        assert!(!message.contains("300MB"), "{message}");
    }

    #[test]
    fn engine_message_is_shown_as_is() {
        let error = SttError::Engine {
            message: "本地语音模型加载失败，模型文件可能已损坏".into(),
        };
        assert_eq!(error.user_message(), "本地语音模型加载失败，模型文件可能已损坏");
    }

    #[test]
    fn known_worker_errors_are_translated() {
        let cases = [
            (
                "inference aborted by the watchdog",
                "语音识别卡住，已中止当前任务并重启引擎。",
            ),
            (
                "worker did not answer within 120s",
                "语音引擎长时间无响应，已重启。",
            ),
            (
                "cannot connect to worker on 127.0.0.1:9: refused",
                "无法连接本地语音引擎，正在重新启动。",
            ),
            (
                "worker exited during startup with exit code: 1",
                "语音引擎启动失败，请检查模型文件后重试。",
            ),
        ];
        for (raw, expected) in cases {
            let error = SttError::Worker {
                message: raw.to_string(),
            };
            assert_eq!(error.user_message(), expected, "原始消息：{raw}");
        }
        // 未覆盖的消息原样返回（避免把中文诊断吞掉）。
        let error = SttError::Worker {
            message: "自定义诊断".into(),
        };
        assert_eq!(error.user_message(), "自定义诊断");
    }

    #[test]
    fn cancelled_config_is_recognized_and_friendly() {
        let error = SttError::Config {
            message: CANCELLED_MESSAGE.into(),
        };
        assert!(error.is_cancelled());
        assert_eq!(error.user_message(), "下载已取消");
    }

    #[test]
    fn non_cancelled_config_is_prefixed() {
        let error = SttError::Config {
            message: "threads must be 1..=16".into(),
        };
        assert!(!error.is_cancelled());
        assert!(error.user_message().starts_with("配置错误："));
    }

    #[test]
    fn reason_flags_match_the_spec() {
        assert!(FailureReason::Network.is_transient());
        assert!(FailureReason::Timeout.is_transient());
        assert!(!FailureReason::Http.is_transient());
        assert!(FailureReason::Storage.is_terminal());
        assert!(FailureReason::Integrity.is_terminal() || !FailureReason::Integrity.is_transient());
    }
}
