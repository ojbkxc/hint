//! 出网代理配置（direct / system / manual 三档）。
//!
//! 参考 `pi-agent-desktop` 的做法：
//! - `direct`：不代理（默认，行为与引入本模块前一致）；
//! - `system`：读 Windows 注册表里「Internet 选项」的代理设置
//!   （`HKCU\...\Internet Settings` 的 `ProxyEnable` / `ProxyServer`），
//!   也就是系统代理软件（Clash / v2rayN 等）开启「系统代理」时写进去的值；
//! - `manual`：用户手填一条 `http://host:port`。
//!
//! 两个必须守住的不变量：
//! 1. **本地回环永不代理**。浮层与本体的 IPC、STT worker 的 HTTP 都走
//!    `127.0.0.1`，一旦被代理接管就整条链路瘫痪（pi-agent-desktop 也为此
//!    显式维护 `LOOPBACK_NO_PROXY_ENTRIES`）。
//! 2. **注册表里的 `ProxyServer` 可能是按协议分开的**（`http=h:p;https=h:p`），
//!    取 https 优先、其次 http，与浏览器行为一致。

use serde::{Deserialize, Serialize};

/// 代理模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// 直连，不读系统代理也不用手填的代理。
    #[default]
    Direct,
    /// 跟随系统（Windows「Internet 选项」里的代理设置）。
    System,
    /// 手动指定。
    Manual,
}

impl ProxyMode {
    /// 用于日志与前端契约的短名。
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyMode::Direct => "direct",
            ProxyMode::System => "system",
            ProxyMode::Manual => "manual",
        }
    }
}

/// 永不走代理的主机（本地回环 + 本机名）。
///
/// 少一个都会出事：`localhost` 与 `127.0.0.1` 在 Windows 上解析结果不一定相同，
/// 而 STT worker 与 IPC 两处都用得上。
pub const LOOPBACK_NO_PROXY: &[&str] = &["localhost", "127.0.0.1", "::1", "[::1]"];

/// 规范化用户手填的代理 URL。
///
/// - 空串 / 纯空白 → `None`；
/// - 缺协议头时补 `http://`（代理软件监听的都是 HTTP CONNECT，不是 https）；
/// - 只接受 http / https / socks5 三种协议，其余当作无效（返回 `None`）。
pub fn normalize_proxy_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let lower = with_scheme.to_ascii_lowercase();
    let supported = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("socks5://");
    if !supported {
        return None;
    }
    // 去掉尾部斜杠：reqwest 的 Proxy::all 期望 host:port 形式，多余斜杠会被当成路径。
    Some(with_scheme.trim_end_matches('/').to_string())
}

/// 从注册表 `reg query` 的输出里解析系统代理 URL。
///
/// 输入形如：
/// ```text
///     ProxyEnable    REG_DWORD    0x1
///     ProxyServer    REG_SZ    http=127.0.0.1:7890;https=127.0.0.1:7890
/// ```
/// `ProxyEnable` 为 0 或缺失 → 无系统代理。
pub fn parse_windows_proxy_registry(output: &str) -> Option<String> {
    let mut enabled = false;
    let mut server: Option<String> = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ProxyEnable") {
            // 形如 `REG_DWORD    0x1`
            let value = rest.split_whitespace().nth(1)?;
            let hex = value.trim_start_matches("0x").trim_start_matches("0X");
            enabled = u32::from_str_radix(hex, 16)
                .map(|v| v != 0)
                .unwrap_or(false);
            continue;
        }
        if let Some(rest) = line.strip_prefix("ProxyServer") {
            // 形如 `REG_SZ    http=127.0.0.1:7890;https=...`
            let mut parts = rest.split_whitespace();
            let _kind = parts.next()?;
            let value = parts.next().unwrap_or("").trim();
            if !value.is_empty() {
                server = Some(value.to_string());
            }
        }
    }
    if !enabled {
        return None;
    }
    let raw = server?;
    // 按协议分开时优先 https，其次 http（与浏览器一致）。
    if let Some(rest) = raw.split(';').find_map(|part| {
        let (scheme, addr) = part.split_once('=')?;
        (scheme.trim().eq_ignore_ascii_case("https")).then_some(addr)
    }) {
        return normalize_proxy_url(rest);
    }
    if let Some(rest) = raw.split(';').find_map(|part| {
        let (scheme, addr) = part.split_once('=')?;
        (scheme.trim().eq_ignore_ascii_case("http")).then_some(addr)
    }) {
        return normalize_proxy_url(rest);
    }
    /// 没写协议名（直接是 host:port）：整串当作 HTTP 代理。
    normalize_proxy_url(&raw)
}

/// 读 Windows「Internet 选项」里的系统代理（同步执行 `reg query`）。
///
/// 只在 `proxy_mode == System` 时调用；失败（无代理软件 / reg 不可用）返回
/// `None`，调用方按直连处理——不能让「探测系统代理失败」变成「应用无法联网」。
pub fn detect_system_proxy() -> Option<String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        // CREATE_NO_WINDOW：避免弹出黑框（GUI 应用里调用命令行通病）。
        #[cfg(target_os = "windows")]
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = Command::new("reg");
        command.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        ]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let output = command.output().ok()?;
        if !output.status.success() {
            return None;
        }
        // 注册表输出是系统 ANSI 代码页，但键值都是 ASCII（ProxyEnable/ProxyServer
        // 与 host:port），lossy 转换足够，不引额外编码依赖。
        let text = String::from_utf8_lossy(&output.stdout);
        parse_windows_proxy_registry(&text)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 按设置算出**生效的代理 URL**（唯一真源，供两个 HTTP 客户端共用）。
///
/// - `direct` → `None`（不代理，行为与引入代理前一致）；
/// - `manual` → 规范化后的手填值；填了但不合法 → `None` 并记 WARN
///   （不静默改成别的模式，避免用户以为「已经用上代理了」）；
/// - `system` → 读注册表；读不到 → `None`（按直连，不阻断使用）。
pub fn effective_proxy_url(mode: ProxyMode, manual_url: &str) -> Option<String> {
    match mode {
        ProxyMode::Direct => None,
        ProxyMode::Manual => {
            let normalized = normalize_proxy_url(manual_url);
            if normalized.is_none() && !manual_url.trim().is_empty() {
                tracing::warn!(
                    raw_len = manual_url.trim().len(),
                    "手填代理地址无法识别（需 http:// 或 socks5:// 开头），本次按直连处理"
                );
            }
            normalized
        }
        ProxyMode::System => {
            let detected = detect_system_proxy();
            if detected.is_none() {
                tracing::debug!("未检测到系统代理，按直连处理");
            }
            detected
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_scheme_and_rejects_garbage() {
        assert_eq!(
            normalize_proxy_url("127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            normalize_proxy_url("http://127.0.0.1:7890/").as_deref(),
            Some("http://127.0.0.1:7890"),
            "尾部斜杠要去掉"
        );
        assert_eq!(
            normalize_proxy_url("socks5://127.0.0.1:1080").as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
        assert_eq!(normalize_proxy_url(""), None);
        assert_eq!(normalize_proxy_url("   "), None);
        assert_eq!(normalize_proxy_url("ftp://x"), None, "不支持的协议");
    }

    #[test]
    fn registry_disabled_means_no_proxy() {
        let out =
            "    ProxyEnable    REG_DWORD    0x0\n    ProxyServer    REG_SZ    127.0.0.1:7890\n";
        assert_eq!(parse_windows_proxy_registry(out), None);
    }

    #[test]
    fn registry_plain_server() {
        let out =
            "    ProxyEnable    REG_DWORD    0x1\n    ProxyServer    REG_SZ    127.0.0.1:7890\n";
        assert_eq!(
            parse_windows_proxy_registry(out).as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn registry_per_protocol_prefers_https() {
        let out = "    ProxyEnable    REG_DWORD    0x1\n    ProxyServer    REG_SZ    http=1.1.1.1:8080;https=2.2.2.2:8443\n";
        assert_eq!(
            parse_windows_proxy_registry(out).as_deref(),
            Some("http://2.2.2.2:8443"),
            "有 https 条目时优先用它"
        );
    }

    #[test]
    fn registry_missing_key_returns_none() {
        assert_eq!(parse_windows_proxy_registry(""), None);
        assert_eq!(
            parse_windows_proxy_registry("    ProxyServer    REG_SZ    127.0.0.1:7890\n"),
            None,
            "没有 ProxyEnable 视为未启用"
        );
    }

    /// 回环必须始终在排除列表里：IPC 与 STT worker 都跑在 127.0.0.1。
    #[test]
    fn loopback_is_always_excluded() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert!(
                LOOPBACK_NO_PROXY.contains(&host),
                "{host} 必须在 NO_PROXY 里"
            );
        }
    }

    #[test]
    fn proxy_mode_names_match_frontend_contract() {
        assert_eq!(ProxyMode::Direct.as_str(), "direct");
        assert_eq!(ProxyMode::System.as_str(), "system");
        assert_eq!(ProxyMode::Manual.as_str(), "manual");
        // 前端传的就是这三个字符串，serde 必须能反序列化。
        assert_eq!(
            serde_json::from_str::<ProxyMode>("\"system\"").expect("应能解析"),
            ProxyMode::System
        );
    }
}
