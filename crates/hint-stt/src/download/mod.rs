//! 下载器：`.part` 断点续传 + 每源重试 + 边下边算 sha256 + 原子 rename。
//!
//! 行为与 cc-haha 的 `downloader.ts` 对齐：
//!
//! | 参数 | 值 |
//! |---|---|
//! | 每源重试 | 5 次 |
//! | 退避 | 500ms 起，指数翻倍，上限 10s |
//! | 无数据判死 | 30s |
//! | 进度回调节流 | 100ms |
//! | 源顺序 | 官方 → hf-mirror，任一失败换下一个 |
//!
//! 哈希覆盖的是网络字节：续传时必须先从 `.part` 已有内容重建摘要状态，
//! 再继续累加新字节。校验失败即删除 `.part`（损坏前缀不可救），
//! 但网络/超时失败**保留** `.part` 供下次续传。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::assets::{self, RemoteFile};
use crate::error::{FailureReason, Result, SttError, from_ureq, scrub_urls};
use crate::manifest;
use crate::types::WorkerConfig;

pub mod disk;

use disk::free_bytes;

/// 每个源的重试次数。
pub const DEFAULT_MAX_RETRIES: u32 = 5;
/// 首次退避。
pub const DEFAULT_BACKOFF: Duration = Duration::from_millis(500);
/// 退避上限。
pub const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// 30 秒收不到任何字节即判死。
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// 进度回调节流。
pub const DEFAULT_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// 安装步骤。SenseVoice 方案没有 `runtime`（静态链接），只剩三个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadStep {
    /// 下载模型文件。
    Model,
    /// 下载后校验（下载器内部完成，进度里只作为瞬时阶段）。
    Verifying,
    /// 下载 VAD。
    Vad,
}

impl DownloadStep {
    /// 展示名。
    pub fn label(self) -> &'static str {
        match self {
            DownloadStep::Model => "model",
            DownloadStep::Verifying => "verifying",
            DownloadStep::Vad => "vad",
        }
    }
}

/// 进度回调负载。
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    /// 当前步骤。
    pub step: DownloadStep,
    /// 当前资源名（例如 `model.int8.onnx`）。
    pub resource: String,
    /// 整个安装任务已完成的字节数（含之前已完成文件）。
    pub completed_bytes: u64,
    /// 整个安装任务的总字节数（241,357,257）。
    pub total_bytes: u64,
    /// 本次续传的起点（字节）；全新下载时是 `None`。
    pub resumed_from_bytes: Option<u64>,
    /// 实际命中的源标签（official / mirror）。
    pub source: String,
}

/// 模型准备状态。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ModelStatus {
    /// 三个文件全部就绪，可随时冷启 worker。
    Ready {
        /// 模型目录（绝对路径）。
        model_dir: String,
    },
    /// 有文件缺失或大小不符，需要 `prepare`。
    Unprepared {
        /// 已存在的 `.part` 字节数（可续传）。
        downloaded_bytes: u64,
        /// 清单总量。
        total_bytes: u64,
    },
    /// 正在下载。
    Downloading {
        /// 当前步骤。
        step: DownloadStep,
        /// 当前资源名。
        resource: String,
        /// 已完成字节。
        completed_bytes: u64,
        /// 总量字节。
        total_bytes: u64,
    },
}

/// 模型管理器：资产落盘、就绪判断、下载编排、删除。
///
/// 所有方法都是阻塞的，调用方应放在专用线程里（Tauri 的 `spawn_blocking`）。
/// 内部状态全部用 `Arc` 共享，克隆后指向同一份下载状态。
#[derive(Clone)]
pub struct ModelManager {
    layout: manifest::Layout,
    /// 当前是否在下载（用于 `status()` 互斥展示）。
    downloading: Arc<AtomicBool>,
    /// 最近一次进度快照，`status()` 据此返回 `Downloading`。
    progress: Arc<std::sync::Mutex<Option<DownloadProgress>>>,
}

impl ModelManager {
    /// `data_root` 是语音数据根（`%APPDATA%\<app>\voice`），不是模型目录本身。
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            layout: manifest::Layout::new(&data_root),
            downloading: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// 模型目录（`<data_root>/sensevoice`）。
    pub fn model_dir(&self) -> &Path {
        &self.layout.base
    }

    /// 目录布局。
    pub fn layout(&self) -> &manifest::Layout {
        &self.layout
    }

    /// 当前状态。只做 `exists + size` 检查，不重算哈希。
    pub fn status(&self) -> ModelStatus {
        let manifest = manifest::read_manifest(&self.layout.manifest);
        if manifest::all_installed(&self.layout, &manifest) {
            return ModelStatus::Ready {
                model_dir: self.layout.base.display().to_string(),
            };
        }
        if self.downloading.load(Ordering::Relaxed) {
            let snapshot = self
                .progress
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone();
            if let Some(progress) = snapshot {
                return ModelStatus::Downloading {
                    step: progress.step,
                    resource: progress.resource,
                    completed_bytes: progress.completed_bytes,
                    total_bytes: progress.total_bytes,
                };
            }
        }
        ModelStatus::Unprepared {
            downloaded_bytes: self.downloaded_bytes(&manifest),
            total_bytes: assets::TOTAL_BYTES,
        }
    }

    /// 是否就绪（快路径）。
    pub fn is_ready(&self) -> bool {
        let manifest = manifest::read_manifest(&self.layout.manifest);
        manifest::all_installed(&self.layout, &manifest)
    }

    /// 已落盘字节数：已完成文件的真实大小 + `.part` 的大小。
    fn downloaded_bytes(&self, manifest: &manifest::Manifest) -> u64 {
        let mut total = 0;
        for file in assets::ALL_FILES {
            if manifest::find_record(manifest, file.target).is_some() {
                if let Ok(meta) = fs::metadata(self.layout.file_path(file.target)) {
                    total += meta.len().min(file.bytes);
                    continue;
                }
            }
            if let Ok(meta) = fs::metadata(self.layout.part_path(file)) {
                total += meta.len().min(file.bytes);
            }
        }
        total
    }

    /// 按当前目录产出 worker 配置（调用前需确认 [`ModelManager::is_ready`]）。
    pub fn worker_config(&self) -> WorkerConfig {
        WorkerConfig::for_model_dir(&self.layout.base)
    }

    /// 下载并校验全部资产，按「模型 → tokens → VAD」顺序。
    ///
    /// * `on_progress`：进度回调，节流到约 100ms 一次；回调必须轻量。
    /// * `cancel`：置为 true 后尽快返回，保留 `.part`。
    ///
    /// 已就绪的文件直接跳过。磁盘预检约 300MB；拿不到可用空间时跳过预检。
    pub fn prepare(
        &self,
        on_progress: impl Fn(DownloadProgress) + Send + Sync,
        cancel: &AtomicBool,
    ) -> Result<()> {
        if self.is_ready() {
            return Ok(());
        }
        if self
            .downloading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(SttError::Config {
                message: "a download is already running".into(),
            });
        }
        let guard = DownloadingGuard(&self.downloading);
        self.store_progress(None);
        let result = self.prepare_inner(on_progress, cancel);
        if let Err(error) = &result {
            // 失败原因必须落日志：以前失败时只清进度，日志里没有任何线索。
            tracing::error!(
                model = assets::MODEL_ID,
                reason = ?error.reason(),
                %error,
                "语音资产准备失败，进度已重置"
            );
            // 失败/取消后清掉进度快照，状态退回 `Unprepared`。
            self.store_progress(None);
        }
        drop(guard);
        result
    }

    fn store_progress(&self, progress: Option<DownloadProgress>) {
        *self
            .progress
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = progress;
    }

    fn prepare_inner(
        &self,
        on_progress: impl Fn(DownloadProgress) + Send + Sync,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.layout.ensure_dirs()?;

        // 只需要为尚未完成的文件预留空间。
        let manifest = manifest::read_manifest(&self.layout.manifest);
        let mut needed = 0u64;
        for file in assets::ALL_FILES {
            if manifest::is_installed(&self.layout, &manifest, file) {
                continue;
            }
            let have = fs::metadata(self.layout.part_path(file))
                .map(|meta| meta.len().min(file.bytes))
                .unwrap_or(0);
            needed += file.bytes.saturating_sub(have);
        }
        if needed > 0 {
            if let Some(free) = free_bytes(&self.layout.base) {
                // 首次安装按清单总量 + 余量预检，语义上就是「约 300MB」；
                // 已有 `.part` 的续传只预留剩余字节 + 64MB 暂存。
                let budget = if needed >= assets::TOTAL_BYTES {
                    assets::REQUIRED_DISK_BYTES
                } else {
                    needed + 64 * 1024 * 1024
                };
                if free < budget {
                    tracing::warn!(
                        free_bytes = free,
                        needed_bytes = budget,
                        "可用磁盘空间不足，取消下载"
                    );
                    // 数字直接写进中文消息：续传时需求远小于首次的约 300MB，
                    // 固定文案会误导用户。
                    return Err(SttError::storage(format!(
                        "磁盘空间不足：当前可用约 {} MB，本次还需要约 {} MB",
                        free / 1024 / 1024,
                        budget / 1024 / 1024
                    )));
                }
            }
        }
        // 已完成文件累加到整体进度里。
        let mut completed = assets::ALL_FILES
            .iter()
            .filter(|file| manifest::is_installed(&self.layout, &manifest, file))
            .map(|file| file.bytes)
            .sum::<u64>();
        let total = assets::TOTAL_BYTES;
        let on_progress = Arc::new(on_progress);

        tracing::info!(
            model = assets::MODEL_ID,
            needed_bytes = needed,
            remaining_installed = completed,
            total_bytes = total,
            "开始准备语音资产"
        );

        for file in assets::ALL_FILES {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            if manifest::is_installed(&self.layout, &manifest, file) {
                continue;
            }
            let step = if file.target == assets::VAD_PATH {
                DownloadStep::Vad
            } else {
                DownloadStep::Model
            };
            let base = completed;
            let report = {
                let listener = Arc::clone(&on_progress);
                let snapshot = Arc::clone(&self.progress);
                let resource = file.name.to_string();
                move |file_done: u64, resumed: Option<u64>, source: &str| {
                    let progress = DownloadProgress {
                        step,
                        resource: resource.clone(),
                        completed_bytes: base + file_done,
                        total_bytes: total,
                        resumed_from_bytes: resumed,
                        source: source.to_string(),
                    };
                    *snapshot.lock().unwrap_or_else(|poison| poison.into_inner()) =
                        Some(progress.clone());
                    listener(progress);
                }
            };

            download_file(self.layout.clone(), file, report, cancel)?;
            completed += file.bytes;
        }

        // 最终确认：任何文件未记录都会被视作未就绪。
        let final_manifest = manifest::read_manifest(&self.layout.manifest);
        if !manifest::all_installed(&self.layout, &final_manifest) {
            return Err(SttError::Integrity {
                message: "asset installation finished but verification is incomplete".into(),
            });
        }
        tracing::info!(
            model = assets::MODEL_ID,
            bytes = assets::TOTAL_BYTES,
            "voice assets prepared"
        );
        Ok(())
    }

    /// 删除整个模型目录（含 `.part` 与 `verified.json`）。幂等。
    pub fn remove(&self) -> Result<()> {
        if self.layout.base.exists() {
            fs::remove_dir_all(&self.layout.base).map_err(|error| {
                SttError::storage(format!(
                    "cannot remove {}: {error}",
                    self.layout.base.display()
                ))
            })?;
        }
        Ok(())
    }

    /// 重算三个文件的 sha256 并刷新 `verified.json`（「校验文件」按钮）。
    pub fn verify(&self) -> Result<()> {
        manifest::verify_files(&self.layout)?;
        let mut records = Vec::with_capacity(assets::ALL_FILES.len());
        for file in assets::ALL_FILES {
            records.push(manifest::VerifiedFile {
                name: file.target.to_string(),
                bytes: file.bytes,
                sha256: file.sha256.to_string(),
            });
        }
        let existing = manifest::read_manifest(&self.layout.manifest);
        manifest::write_manifest(
            &self.layout.manifest,
            &manifest::merge_records(&existing, &records),
        )
    }
}

struct DownloadingGuard<'a>(&'a AtomicBool);

impl Drop for DownloadingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn cancelled() -> SttError {
    SttError::Config {
        message: CANCELLED_MESSAGE.into(),
    }
}

// ---------------------------------------------------------------------------
// 单文件下载
// ---------------------------------------------------------------------------

/// 统一 Agent：不读环境代理变量（避免 hf.co 走错代理），30 秒读超时兜底。
///
/// 未启用 `proxy-from-env`，`AgentBuilder` 不会自动读取 `HTTP(S)_PROXY`；
/// 只启用 `native-tls` feature 时，`ureq` 不会自动选择 TLS 后端，必须显式注入
/// 系统 TLS 连接器（Windows 上是 schannel，无需 C 工具链）。
fn agent() -> Result<ureq::Agent> {
    let tls = ureq::native_tls::TlsConnector::new().map_err(|error| {
        SttError::network(
            FailureReason::Certificate,
            format!("cannot initialize the system TLS connector: {error}"),
            None,
        )
    })?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(DEFAULT_IDLE_TIMEOUT)
        .redirects(10)
        .build())
}

/// 下载单个文件。成功返回最终命中的源标签。
fn download_file<F>(
    layout: manifest::Layout,
    file: &RemoteFile,
    progress: F,
    cancel: &AtomicBool,
) -> Result<()>
where
    F: Fn(u64, Option<u64>, &str),
{
    let destination = layout.file_path(file.target);
    let part = layout.part_path(file);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            SttError::storage(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let urls = assets::resolve_urls(file);
    let mut last_error: Option<SttError> = None;

    tracing::info!(
        resource = file.name,
        bytes = file.bytes,
        sources = urls.len(),
        part_bytes = fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0),
        "开始下载语音资产"
    );

    // 与 `resolve_urls` 的顺序一一对应；进度里用短标签（official / mirror），
    // 不是 URL——前端据此显示「官方源 / 镜像源」，也是进度结构体文档的约定。
    for (origin, url) in assets::ORIGINS.iter().zip(urls) {
        let source = origin.label;
        for retry in 0..=DEFAULT_MAX_RETRIES {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            match transfer(&part, &destination, file, &url, &source, &progress, cancel) {
                Ok(true) => {
                    let record = manifest::VerifiedFile {
                        name: file.target.to_string(),
                        bytes: file.bytes,
                        sha256: file.sha256.to_string(),
                    };
                    manifest::record_verified(&layout.manifest, record)?;
                    tracing::info!(resource = file.name, source = %source, "asset installed");
                    return Ok(());
                }
                // `.part` 已被丢弃（416 / Content-Range 不一致 / 坏哈希前缀）：
                // 立刻从零重来一次，不计入重试次数。
                Ok(false) => {
                    tracing::warn!(resource = file.name, source = %source, "restarting download from zero");
                    match transfer(&part, &destination, file, &url, &source, &progress, cancel) {
                        Ok(true) => {
                            let record = manifest::VerifiedFile {
                                name: file.target.to_string(),
                                bytes: file.bytes,
                                sha256: file.sha256.to_string(),
                            };
                            manifest::record_verified(&layout.manifest, record)?;
                            tracing::info!(resource = file.name, source = %source, "asset installed after restart");
                            return Ok(());
                        }
                        Ok(false) => {
                            // 连续两次都要求「丢弃 .part 从零重来」：这是服务端
                            // 协议异常或源侧问题，按可重试失败处理（计入重试并
                            // 最终换源），而不是当作终态 integrity 直接失败。
                            tracing::warn!(resource = file.name, source = %source, "download keeps restarting from zero");
                            last_error = Some(SttError::network(
                                FailureReason::Network,
                                format!("download of {} kept restarting from zero", file.name),
                                None,
                            ));
                            if retry == DEFAULT_MAX_RETRIES {
                                break; // 换下一个源
                            }
                            sleep_backoff(retry, cancel)?;
                        }
                        Err(retry_error) => {
                            let reason = retry_error.reason();
                            if reason.is_terminal() || error_is_cancelled(&retry_error) {
                                return Err(retry_error);
                            }
                            last_error = Some(retry_error);
                            if !reason.is_transient() || retry == DEFAULT_MAX_RETRIES {
                                break; // 换下一个源
                            }
                            sleep_backoff(retry, cancel)?;
                        }
                    }
                }
                Err(error) => {
                    let reason = error.reason();
                    // 记录真实失败原因：以前只有成功日志，下载失败时日志里一片空白，
                    // 无法区分 DNS / 证书 / 超时 / HTTP 状态码。
                    tracing::warn!(
                        resource = file.name,
                        source = %source,
                        retry,
                        reason = ?reason,
                        %error,
                        "下载尝试失败"
                    );
                    if reason.is_terminal() || error_is_cancelled(&error) {
                        return Err(error);
                    }
                    last_error = Some(error);
                    if !reason.is_transient() || retry == DEFAULT_MAX_RETRIES {
                        break; // 换下一个源
                    }
                    sleep_backoff(retry, cancel)?;
                }
            }
        }
    }

    let error = last_error.unwrap_or_else(|| {
        SttError::network(
            FailureReason::Unknown,
            format!("no download source configured for {}", file.name),
            None,
        )
    });
    tracing::error!(
        resource = file.name,
        reason = ?error.reason(),
        %error,
        "语音资产下载失败，所有下载源均已尝试"
    );
    Err(error)
}

fn error_is_cancelled(error: &SttError) -> bool {
    error.is_cancelled()
}

/// 指数退避：500ms 起翻倍，封顶 10s；退避期间响应取消。
fn sleep_backoff(retry: u32, cancel: &AtomicBool) -> Result<()> {
    let backoff = DEFAULT_BACKOFF
        .saturating_mul(1u32 << retry.min(5))
        .min(MAX_BACKOFF);
    let deadline = Instant::now() + backoff;
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// 取消时使用的固定消息，便于在重试循环里识别。
const CANCELLED_MESSAGE: &str = crate::error::CANCELLED_MESSAGE;

/// 一次传输。
///
/// 返回值语义：
/// - `Ok(true)`：文件已校验并原子落到目标位置；
/// - `Ok(false)`：`.part` 已被丢弃，调用方应立即从零重来一次；
/// - `Err(..)`：本次失败，按分类决定重试同源 / 换源 / 立即失败。
#[allow(clippy::too_many_arguments)]
fn transfer<F>(
    part: &Path,
    destination: &Path,
    file: &RemoteFile,
    url: &str,
    source: &str,
    progress: &F,
    cancel: &AtomicBool,
) -> Result<bool>
where
    F: Fn(u64, Option<u64>, &str),
{
    let mut offset = part_len(part)?;
    if offset > file.bytes {
        remove_quietly(part);
        offset = 0;
    }

    // 完整的 .part：只需重算 hash 并 rename。
    if offset == file.bytes {
        let digest = hash_prefix(part, offset)?;
        if !digest.eq_ignore_ascii_case(file.sha256) {
            remove_quietly(part);
            return Ok(false);
        }
        rename_into_place(part, destination)?;
        progress(file.bytes, Some(offset), source);
        return Ok(true);
    }

    let request = agent()?.get(url).set("Accept-Encoding", "identity");
    let request = if offset > 0 {
        request.set("Range", &format!("bytes={offset}-"))
    } else {
        request
    };

    // `ureq` 的读超时是每次 read 的超时，天然实现「30 秒无数据判死」。
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(416, _)) if offset > 0 => {
            // Range 不被接受：丢掉 .part，由调用方从零重来。
            remove_quietly(part);
            return Ok(false);
        }
        Err(error) => return Err(from_ureq(error, file.name)),
    };

    let status = response.status();
    let mut resumed = false;
    match status {
        206 => {
            // 校验 Content-Range 与清单一致。
            let content_range = response.header("content-range").map(str::to_string);
            let consistent = parse_content_range(content_range.as_deref()).is_some_and(|range| {
                offset > 0
                    && range.start == offset
                    && range.end == file.bytes - 1
                    && range.total.is_none_or(|total| total == file.bytes)
            });
            if !consistent {
                remove_quietly(part);
                return Ok(false);
            }
            resumed = true;
        }
        200 => {
            if let Some(length) = response.header("content-length") {
                if length.parse::<u64>().ok() != Some(file.bytes) {
                    return Err(SttError::Integrity {
                        message: format!(
                            "server reported {length} bytes for {}, expected {}",
                            file.name, file.bytes
                        ),
                    });
                }
            }
            // 服务器不支持 Range：顺序覆盖已有 .part。
            offset = 0;
        }
        other => {
            return Err(SttError::network(
                FailureReason::Http,
                format!("HTTP {other} from {source} while fetching {}", file.name),
                Some(other),
            ));
        }
    }

    let mut hasher = if resumed {
        // 续传：从 .part 已有字节重建哈希状态，而不是只算新下载的部分。
        // 读不到前缀（磁盘错误 / 文件被占用）时必须丢弃 .part 从零重来：
        // 若从零状态继续累加新字节，哈希与真实内容不符，会把文件写坏。
        match hash_state_from_prefix(part, offset) {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(%error, "cannot rebuild hash state, discarding .part");
                remove_quietly(part);
                return Ok(false);
            }
        }
    } else {
        Sha256::new()
    };
    let mut handle = if resumed {
        OpenOptions::new()
            .append(true)
            .open(part)
            .map_err(|error| {
                SttError::storage(format!("cannot append {}: {error}", part.display()))
            })?
    } else {
        File::create(part).map_err(|error| {
            SttError::storage(format!("cannot create {}: {error}", part.display()))
        })?
    };

    let mut completed = offset;
    // 首次必须立即上报一次进度，所以把「上次上报时间」回拨一个节流周期。
    let mut last_report = Instant::now()
        .checked_sub(DEFAULT_PROGRESS_INTERVAL)
        .unwrap_or_else(Instant::now);
    progress(completed, if resumed { Some(offset) } else { None }, source);

    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 128 * 1024];
    let result = (|| -> Result<()> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            let read = reader.read(&mut buffer).map_err(|error| {
                let reason = crate::error::classify_io_error(&error);
                SttError::network(
                    reason,
                    format!("read failed for {}: {error}", file.name),
                    None,
                )
            })?;
            if read == 0 {
                break;
            }
            if completed + read as u64 > file.bytes {
                return Err(SttError::Integrity {
                    message: format!(
                        "received more than the expected {} bytes for {}",
                        file.bytes, file.name
                    ),
                });
            }
            hasher.update(&buffer[..read]);
            write_fully(&mut handle, &buffer[..read]).map_err(|error| {
                SttError::storage(format!("cannot write {}: {error}", part.display()))
            })?;
            completed += read as u64;
            if last_report.elapsed() >= DEFAULT_PROGRESS_INTERVAL || completed == file.bytes {
                last_report = Instant::now();
                progress(completed, if resumed { Some(offset) } else { None }, source);
            }
        }
        if completed < file.bytes {
            return Err(SttError::network(
                FailureReason::Network,
                format!(
                    "connection closed after {completed} of {} bytes for {}",
                    file.bytes, file.name
                ),
                None,
            ));
        }
        Ok(())
    })();
    drop(handle);
    if let Err(error) = result {
        return Err(error);
    }

    // 校验：哈希覆盖网络字节，已完成全部字节。
    let digest = hasher.finalize();
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    let actual = assets::encode_sha256(&array);
    if !actual.eq_ignore_ascii_case(file.sha256) {
        remove_quietly(part);
        if resumed {
            // 旧 .part 的坏前缀可救：从零再来一次。
            return Ok(false);
        }
        // 全新下载仍然不对：内容就是错的，换源也无意义。
        return Err(SttError::Integrity {
            message: format!("sha256 mismatch for {}", file.name),
        });
    }

    rename_into_place(part, destination)?;
    progress(
        file.bytes,
        if resumed { Some(offset) } else { None },
        source,
    );
    Ok(true)
}

/// 把 `chunk` 全部写完；`File::write_all` 已保证这一点，这里保留封装以便统一错误。
fn write_fully(handle: &mut File, chunk: &[u8]) -> std::io::Result<()> {
    handle.write_all(chunk)?;
    Ok(())
}

/// 某些文件系统上 rename 覆盖已有目标会失败（Windows 一般允许）。失败时先删目标再改名。
fn rename_into_place(part: &Path, destination: &Path) -> Result<()> {
    if let Err(error) = fs::rename(part, destination) {
        remove_quietly(destination);
        fs::rename(part, destination).map_err(|retry| {
            SttError::storage(format!(
                "cannot move {} into place (first error: {error}; retry error: {retry})",
                part.display()
            ))
        })?;
    }
    Ok(())
}

fn part_len(part: &Path) -> Result<u64> {
    match fs::metadata(part) {
        Ok(meta) => Ok(meta.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(SttError::storage(format!(
            "cannot stat {}: {error}",
            part.display()
        ))),
    }
}

fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}

/// 计算 `.part` 前 `length` 字节的 sha256，返回 hex。
fn hash_prefix(path: &Path, length: u64) -> Result<String> {
    let hash = hash_state_from_prefix(path, length)?;
    let digest = hash.finalize();
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    Ok(assets::encode_sha256(&array))
}

/// 从 `.part` 前 `length` 字节重建哈希状态（续传的关键）。
fn hash_state_from_prefix(path: &Path, length: u64) -> Result<Sha256> {
    let mut file = File::open(path)
        .map_err(|error| SttError::storage(format!("cannot open {}: {error}", path.display())))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| SttError::storage(format!("cannot seek {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut remaining = length;
    let mut buffer = vec![0u8; 256 * 1024];
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..want]).map_err(|error| {
            SttError::storage(format!("cannot read {}: {error}", path.display()))
        })?;
        hasher.update(&buffer[..want]);
        remaining -= want as u64;
    }
    Ok(hasher)
}

/// `Content-Range` 的解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    /// 起始字节（含）。
    pub start: u64,
    /// 结束字节（含）。
    pub end: u64,
    /// 文件总长度；`*` 时为 `None`。
    pub total: Option<u64>,
}

/// 解析 `bytes <start>-<end>/<total|*>`。格式不符返回 `None`。
pub fn parse_content_range(header: Option<&str>) -> Option<ContentRange> {
    let value = header?.trim();
    let rest = value.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    if end < start {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        other => Some(other.parse::<u64>().ok()?),
    };
    Some(ContentRange { start, end, total })
}

/// 下载失败时的用户提示（包含分类文案 + 脱敏细节）。
pub fn user_message_for(error: &SttError) -> String {
    // 用 user_message() 而不是 Display：后者是英文枚举前缀，
    // 会把「模型文件损坏」这类已诊断清楚的原因显示成「未知错误」。
    scrub_urls(&error.user_message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_part_with_bad_hash_is_discarded_without_network() {
        // `.part` 大小已等于清单字节数、但内容不对：transfer 应重算哈希、
        // 发现不符后删除 `.part` 并返回 Ok(false)，不发起任何网络请求。
        let root = std::env::temp_dir().join(format!("hint-stt-badpart-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let layout = manifest::Layout::new(&root);
        layout.ensure_dirs().unwrap();
        let payload = b"this is not the real model";
        let file = RemoteFile {
            name: "fake.onnx",
            target: "models/fake.onnx",
            bytes: payload.len() as u64,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            repo: "r",
            revision: "v",
        };
        let part = layout.part_path(&file);
        fs::write(&part, payload).unwrap();
        let destination = layout.file_path(file.target);
        let cancel = AtomicBool::new(false);
        let result = transfer(
            &part,
            &destination,
            &file,
            "https://example.invalid/x",
            "test",
            &|_, _, _| {},
            &cancel,
        )
        .unwrap();
        assert!(!result, "bad complete part must request a restart");
        assert!(!part.exists(), ".part with bad hash must be removed");
        assert!(!destination.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_range_parsing() {
        let parsed = parse_content_range(Some("bytes 100-239233840/239233841")).unwrap();
        assert_eq!(parsed.start, 100);
        assert_eq!(parsed.end, 239_233_840);
        assert_eq!(parsed.total, Some(239_233_841));
        assert_eq!(
            parse_content_range(Some("bytes 0-9/*")).unwrap(),
            ContentRange {
                start: 0,
                end: 9,
                total: None
            }
        );
        assert!(parse_content_range(Some("bytes 9-0/10")).is_none());
        assert!(parse_content_range(Some("items 0-9/10")).is_none());
        assert!(parse_content_range(Some("bytes 0-9")).is_none());
        assert!(parse_content_range(None).is_none());
    }

    #[test]
    fn hash_state_rebuilds_from_existing_part() {
        let dir = std::env::temp_dir().join(format!("hint-stt-resume-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.bin");
        let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &bytes[..70_000]).unwrap();

        let resumed = hash_prefix(&path, 70_000).unwrap();
        let mut expected = Sha256::new();
        expected.update(&bytes[..70_000]);
        let expected_hex = assets::encode_sha256(&expected.finalize().into());
        assert_eq!(resumed, expected_hex);

        // 续传后追加的字节与一次性哈希一致。
        let mut hasher = hash_state_from_prefix(&path, 70_000).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        write_fully(&mut file, &bytes[70_000..]).unwrap();
        hasher.update(&bytes[70_000..]);
        let full = assets::encode_sha256(&hasher.finalize().into());
        let mut whole = Sha256::new();
        whole.update(&bytes);
        assert_eq!(full, assets::encode_sha256(&whole.finalize().into()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manager_reports_unprepared_for_an_empty_root() {
        let root = std::env::temp_dir().join(format!("hint-stt-mm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let manager = ModelManager::new(root.clone());
        let status = manager.status();
        assert!(
            matches!(status, ModelStatus::Unprepared { total_bytes, .. } if total_bytes == assets::TOTAL_BYTES)
        );
        assert!(!manager.is_ready());
        // remove 幂等
        manager.remove().unwrap();
        manager.remove().unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ready_status_only_checks_size() {
        let root = std::env::temp_dir().join(format!("hint-stt-ready-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let manager = ModelManager::new(root.clone());
        manager.layout().ensure_dirs().unwrap();

        // 造三个「大小正确」的文件（内容不对，但只有大小参与判断）。
        let mut records = Vec::new();
        for file in assets::ALL_FILES {
            let path = manager.layout().file_path(file.target);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let handle = File::create(&path).unwrap();
            handle.set_len(file.bytes).unwrap();
            records.push(manifest::VerifiedFile {
                name: file.target.to_string(),
                bytes: file.bytes,
                sha256: file.sha256.to_string(),
            });
        }
        manifest::write_manifest(
            &manager.layout().manifest,
            &manifest::Manifest {
                version: manifest::MANIFEST_VERSION,
                files: records,
            },
        )
        .unwrap();
        assert!(manager.is_ready());
        assert!(matches!(manager.status(), ModelStatus::Ready { .. }));

        // 重算 hash 会发现内容不对。
        assert!(manager.verify().is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
