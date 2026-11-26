//! 启动时检查更新（强制）。
//!
//! ## 为什么自己写而不用 `tauri-plugin-updater`
//!
//! 官方插件的 `latest.json` 流程要求每个平台产物配 minisign 签名，且签名必须由
//! 私钥在 CI 里生成、公钥编译进二进制。本项目走的是 NSIS 安装包（`/S` 可静默安装），
//! 用 GitHub Releases 的 assets 直接下载安装包就够，不需要额外维护一对密钥与签名
//! 文件——少一个会在发版时忘掉、然后「自动更新悄悄失效」的环节。
//!
//! ## 与其他网络请求的区别
//!
//! 更新检查访问的是 **github.com / objects.githubusercontent.com**，与模型接口
//! 完全不同路径。用户的代理设置（Clash 等）常常只代理模型域名、不代理 GitHub，
//! 实测过直连 GitHub 反而更快（11.9 秒下完 8.4 MB，走本地代理 120 秒超时）。
//! 所以这里**默认直连**，代理只作为兜底尝试。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// 仓库坐标（固定，不从配置读——用户改不了自己的更新源才有意义）。
const REPO: &str = "ojbkxc/hint";

/// 检查更新的整体超时。启动路径上不能让用户干等。
const CHECK_TIMEOUT: Duration = Duration::from_secs(12);

/// 下载安装包的超时（8.4 MB 在慢网下也要给足）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

/// 发现的新版本。
///
/// 需要 `Serialize`：它直接作为 Tauri 命令的返回值过 IPC。字段统一 camelCase
/// 以匹配前端（`installerUrl` / `installerName`）——两端命名不一致时前端会拿到
/// `undefined` 却仍然「请求成功」，是最难查的一类错。
///
/// 同时需要 `Deserialize`：`download_update(info)` 要把前端传回的对象读回来。
/// `rename_all` 对两个方向都生效，所以 derive 一次即可，不必手写 impl。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// 最新版本号（去掉前缀 `v`），如 `1.1.32`。
    pub version: String,
    /// 安装包下载地址。
    pub installer_url: String,
    /// 安装包文件名（用于落盘）。
    pub installer_name: String,
    /// 该版本的发布说明（Markdown 原文）。
    #[serde(default)]
    pub notes: String,
}

/// GitHub Releases API 的响应里我们用到的字段。
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// 把 `v1.2.3` / `1.2.3` 解析成可比较的三段数字。
///
/// 解析失败返回 `None`（调用方据此跳过更新，绝不在版本号格式异常时把用户卡在
/// 「强制更新」界面里）。
fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    // 第三段可能带后缀（1.2.3-beta），取数字前缀。
    let patch_raw = parts.next()?;
    let digits: String = patch_raw
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let patch = digits.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// 新版本是否比当前版本更新。
///
/// 只认「严格更大」：相同或更旧都不更新，避免用户手动装了测试版之后被「降级」
/// 回正式版。
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        // 任何一边解析不出来都不更新（宁可漏更，不可卡死）。
        _ => false,
    }
}

/// 从 release 响应里挑出当前平台的安装包。
///
/// 只认 `_x64-setup.exe`：这是 NSIS 安装包，支持 `/S` 静默安装。`.exe`（裸程序）
/// 与 `.msi` 都不能原地替换正在运行的自身。
fn pick_installer(release: &Release) -> Option<&Asset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with("_x64-setup.exe"))
}

/// 构造 HTTP 客户端。默认直连，代理只作兜底。
fn build_client(proxy: Option<&str>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .user_agent("hint-updater");
    if let Some(url) = proxy.filter(|url| !url.trim().is_empty()) {
        match reqwest::Proxy::all(url) {
            Ok(parsed) => builder = builder.proxy(parsed),
            // 代理地址解析失败就直连——更新网络路径不该因为一条坏配置而失效。
            Err(error) => {
                tracing::warn!(%error, "更新检查的代理地址无法解析，改用直连");
                builder = builder.no_proxy();
            }
        }
    } else {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|error| format!("创建网络客户端失败：{error}"))
}

/// 查询 GitHub 上是否有比 `current` 更新的版本。
///
/// 返回 `Ok(None)` 表示「已是最新」或「无法判断」——**所有失败都退化成不更新**，
/// 因为它在启动路径上：网络不通、被墙、GitHub 挂了都不该让用户打不开软件。
pub async fn check_latest(
    current: &str,
    proxy: Option<&str>,
) -> Result<Option<UpdateInfo>, String> {
    let client = build_client(proxy)?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("查询更新失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("查询更新失败：HTTP {}", response.status()));
    }
    let release: Release = response
        .json()
        .await
        .map_err(|error| format!("解析更新信息失败：{error}"))?;

    if !is_newer(&release.tag_name, current) {
        tracing::info!(
            latest = %release.tag_name,
            current,
            "已是最新版本"
        );
        return Ok(None);
    }

    let Some(asset) = pick_installer(&release) else {
        // 有新版但没有可用的安装包：不要卡住用户，只记一条日志。
        tracing::warn!(
            latest = %release.tag_name,
            assets = ?release.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            "发现新版本但没有 x64 安装包，跳过自动更新"
        );
        return Ok(None);
    };

    Ok(Some(UpdateInfo {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        installer_url: asset.browser_download_url.clone(),
        installer_name: asset.name.clone(),
        notes: release.body,
    }))
}

/// 下载安装包到 `dest`，返回实际写入的字节数。
///
/// 先写 `.part` 再改名：下载中断留下的半个文件不能被误当成可用安装包。
pub async fn download_installer(
    info: &UpdateInfo,
    dest: &Path,
    proxy: Option<&str>,
) -> Result<u64, String> {
    let client = build_client(proxy)?;
    let mut response = client
        .get(&info.installer_url)
        // 下载不受检查的超时限制，用单独的读超时。
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("下载安装包失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载安装包失败：HTTP {}", response.status()));
    }

    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|error| format!("创建下载目录失败：{error}"))?;
    }
    let part = dest.with_extension("part");
    let mut file =
        std::fs::File::create(&part).map_err(|error| format!("创建文件失败：{error}"))?;
    let mut written = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("下载中断：{error}"))?
    {
        file.write_all(&chunk)
            .map_err(|error| format!("写入安装包失败：{error}"))?;
        written += chunk.len() as u64;
    }
    file.flush()
        .map_err(|error| format!("刷新安装包失败：{error}"))?;
    drop(file);

    // 空文件说明服务端给了 0 字节：绝不能当成下载成功。
    if written == 0 {
        let _ = std::fs::remove_file(&part);
        return Err("下载到的安装包是空文件".to_string());
    }
    std::fs::rename(&part, dest).map_err(|error| format!("保存安装包失败：{error}"))?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_basic() {
        assert!(is_newer("1.1.32", "1.1.31"));
        assert!(is_newer("v1.2.0", "1.1.99"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(!is_newer("1.1.31", "1.1.31"));
        // 更旧的不更新（用户手动装了测试版不该被降级）。
        assert!(!is_newer("1.1.30", "1.1.31"));
    }

    #[test]
    fn version_compare_handles_prerelease_suffix() {
        // 第三段带后缀时取数字前缀。
        assert!(is_newer("1.1.32-beta", "1.1.31"));
        assert!(!is_newer("1.1.32-beta", "1.1.32"));
    }

    #[test]
    fn version_compare_rejects_garbage() {
        // 解析不出来一律不更新——启动路径上不能因为格式异常卡住用户。
        assert!(!is_newer("abc", "1.1.31"));
        assert!(!is_newer("1.1.32", "def"));
        assert!(!is_newer("", "1.1.31"));
        assert!(!is_newer("1.1", "1.1.31"));
    }

    #[test]
    fn picks_only_nsis_installer() {
        let release = Release {
            tag_name: "v1.1.32".into(),
            body: "notes".into(),
            assets: vec![
                Asset {
                    name: "hint.exe".into(),
                    browser_download_url: "https://example.com/hint.exe".into(),
                },
                Asset {
                    name: "hint_1.1.32_x64-setup.exe".into(),
                    browser_download_url: "https://example.com/setup.exe".into(),
                },
            ],
        };
        let asset = pick_installer(&release).expect("应挑到 setup");
        assert_eq!(asset.name, "hint_1.1.32_x64-setup.exe");
    }

    #[test]
    fn no_installer_means_no_update() {
        let release = Release {
            tag_name: "v1.1.32".into(),
            body: String::new(),
            assets: vec![Asset {
                name: "hint.exe".into(),
                browser_download_url: "https://example.com/hint.exe".into(),
            }],
        };
        assert!(pick_installer(&release).is_none());
    }

    #[test]
    fn installer_name_is_reported_for_saving() {
        let release = Release {
            tag_name: "v1.1.32".into(),
            body: "## 更新".into(),
            assets: vec![Asset {
                name: "hint_1.1.32_x64-setup.exe".into(),
                browser_download_url: "https://example.com/x".into(),
            }],
        };
        let asset = pick_installer(&release).unwrap();
        assert_eq!(asset.name, "hint_1.1.32_x64-setup.exe");
    }
}
