//! 日志落盘：stderr 与 `%APPDATA%\com.ojbkxc.hint\logs\hint.log` 双写。
//!
//! 不引入额外 crate：这里手写一个 `std::io::Write`，尺寸超过 5MB 时把
//! `hint.log` 轮转为 `hint.1.log`（只保留一份历史）。日志目录创建失败
//! 只退化到 stderr，不让应用启动失败。

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 单个日志文件的上限（约 5MB）。
const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct FileSlot {
    /// `None` 只在轮转的瞬间存在（Windows 必须先关闭文件才能改名）。
    file: Option<std::fs::File>,
    size: u64,
    path: PathBuf,
    backup: PathBuf,
    /// 轮转失败后不再每次写入都重试（避免每条日志都 sync/rename 失败）。
    rotate_failed: bool,
}

impl FileSlot {
    fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("hint.log");
        let backup = dir.join("hint.1.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            file: Some(file),
            size,
            path,
            backup,
            rotate_failed: false,
        })
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if self.size + buf.len() as u64 > MAX_BYTES && !self.rotate_failed {
            self.rotate();
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("日志文件不可用"))?;
        file.write_all(buf)?;
        self.size = self.size.saturating_add(buf.len() as u64);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }

    /// 轮转失败（例如杀毒软件占用）时不丢当前日志：继续往原文件追加。
    fn rotate(&mut self) {
        let Some(file) = self.file.take() else {
            return;
        };
        let _ = file.sync_all();
        drop(file);
        let _ = std::fs::remove_file(&self.backup);
        // rename 失败时原文件还在，后续仍追加到它，尺寸计数保持不变。
        let renamed = std::fs::rename(&self.path, &self.backup).is_ok();
        if !renamed {
            self.rotate_failed = true;
        }
        let previous_size = self.size;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                self.file = Some(file);
                self.size = if renamed { 0 } else { previous_size };
            }
            Err(error) => {
                // 新文件都建不出来时只能把原文件接回去，保证后续日志不丢。
                self.file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                    .ok();
                if renamed {
                    self.size = 0;
                }
                if self.file.is_none() {
                    eprintln!("日志轮转后无法重新打开文件：{error}");
                }
            }
        }
    }
}

struct RotatingFile {
    slot: Mutex<FileSlot>,
}

impl RotatingFile {
    fn open(dir: &Path) -> io::Result<Self> {
        Ok(Self {
            slot: Mutex::new(FileSlot::open(dir)?),
        })
    }

    fn write_bytes(&self, buf: &[u8]) -> io::Result<()> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.write_all(buf)
    }

    fn flush(&self) -> io::Result<()> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.flush()
    }
}

/// 传给 `tracing` 的写入口；每次事件克隆一个轻量句柄。
#[derive(Clone)]
struct RotatingWriter {
    inner: Arc<RotatingFile>,
}

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write_bytes(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("hint=info"))
}

/// 初始化全局日志：stderr + 文件。必须在拿到 app_data_dir 之后调用。
pub(crate) fn init(log_dir: &Path) {
    match RotatingFile::open(log_dir) {
        Ok(file) => {
            let writer = Arc::new(file);
            let file_writer = Arc::clone(&writer);
            let result = tracing_subscriber::registry()
                .with(env_filter())
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(false)
                        .with_writer(std::io::stderr),
                )
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(false)
                        .with_ansi(false)
                        .with_writer(move || RotatingWriter {
                            inner: Arc::clone(&file_writer),
                        }),
                )
                .try_init();
            if result.is_err() {
                eprintln!("日志订阅者已初始化，跳过重复初始化");
                return;
            }
            tracing::info!(path = %log_dir.join("hint.log").display(), "日志已写入文件");
        }
        Err(error) => {
            eprintln!(
                "无法创建日志文件（{}）：{error}；本次仅输出到 stderr",
                log_dir.display()
            );
            let _ = tracing_subscriber::fmt()
                .with_env_filter(env_filter())
                .with_target(false)
                .try_init();
        }
    }
}
