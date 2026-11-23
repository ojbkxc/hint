//! 父进程侧的 sidecar 管理：启动自身 exe（`--voice-worker`）、读端口、发请求、回收。
//!
//! 设计取舍与 cc-haha 一致：
//! - **自唤**：不额外分发 sidecar exe，而是 `spawn(当前进程, ["--voice-worker"])`，
//!   显式 `env_clear()` 后只注入白名单变量，避免代理/密钥泄漏进子进程。
//! - **stdin 即心跳**：worker 监听 stdin EOF（Windows 上没有 SIGTERM）。
//!   `stop()` 直接关 stdin 并等待退出；超时后 `kill()`（TerminateProcess）。
//! - **crash-only**：请求失败、超时或进程退出后一律丢弃 worker，
//!   下一次请求冷启，不复用可能已脏的 native 状态。
//! - **空闲回收交给调用方**：本类型只提供 [`WorkerClient::last_used`] /
//!   [`WorkerClient::idle_for`]，由上层调度器决定何时 `stop()`。
//!
//! 请求串行化由内部 `Mutex` 保证：native 推理是同步的，队列保证同一时刻
//! 只有一个请求，避免多线程争抢 1–3GB 内存。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use crate::error::{Result, SttError};
use crate::types::{
    Transcript, WORKER_CONFIG_ENV, WORKER_TOKEN_ENV, WorkerConfig, WorkerErrorBody,
};

pub use crate::types::WorkerConfig as Config;

/// 启动超时：包含加载 239MB 模型的时间。
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// 单次推理超时。
pub const INFERENCE_TIMEOUT: Duration = Duration::from_secs(120);
/// 关闭 stdin 后等待退出的宽限。
pub const STOP_GRACE: Duration = Duration::from_secs(1);
/// stdout 首行上限。
const READY_LINE_LIMIT: usize = 4096;
/// 响应体上限。
const RESPONSE_LIMIT: usize = 128 * 1024;
/// 可中断读取的轮询间隔：每次最多阻塞这么久，随后检查 abort 标志。
const READ_POLL: Duration = Duration::from_millis(250);
/// worker 传进来的命令行开关。
pub const WORKER_FLAG: &str = "--voice-worker";

/// worker 启动时注入的环境变量白名单。
///
/// 与 cc-haha 相同：不继承任何密钥与代理变量——代理会让 loopback 请求走代理，
/// 密钥变量等于直接送给子进程。
const ENV_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SystemRoot",
    "SYSTEMROOT",
    "LANG",
    "LC_ALL",
    "LD_LIBRARY_PATH",
];

/// `WorkerClient` 的内部运行时状态。
struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    port: u16,
    /// 本次进程的 Bearer token。
    auth: String,
}

/// 一个活的（或可重启的）worker 句柄。
pub struct WorkerClient {
    /// 用于 spawn 的可执行文件；通常是 `std::env::current_exe()`。
    exe: PathBuf,
    /// 传给子进程的配置：序列化进环境变量。
    config: WorkerConfig,
    /// 工作目录。
    cwd: PathBuf,
    /// 串行化推理请求。
    state: Mutex<Option<Running>>,
    /// 上次使用时间；`None` 表示从未成功请求。
    last_used: Mutex<Option<Instant>>,
    /// 中断标志：置位后正在进行的推理会在下一次轮询时中止并回收 worker。
    ///
    /// 这只是取消**推理**（放弃当前请求），不改变 `transcribe` 的锁语义；
    /// 看门狗用它打断可能已挂死的长时间推理。
    abort: AtomicBool,
}

impl WorkerClient {
    /// 启动 worker：`spawn(exe, [--voice-worker])`，等待 stdout 首行 `{"port":N}`。
    ///
    /// `config` 的路径字段必须是绝对路径；`exe_path` 一般传
    /// [`std::env::current_exe()`]。工作目录取模型目录的上一级（`<data_root>/sensevoice`）。
    pub fn spawn(exe_path: &Path, config: WorkerConfig) -> Result<Self> {
        let cwd = config
            .model_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::spawn_in(exe_path, config, cwd)
    }

    /// 显式指定工作目录的版本。
    pub fn spawn_in(
        exe_path: &Path,
        config: WorkerConfig,
        cwd: impl Into<PathBuf>,
    ) -> Result<Self> {
        config.validate()?;
        let client = Self {
            exe: exe_path.to_path_buf(),
            config,
            cwd: cwd.into(),
            state: Mutex::new(None),
            last_used: Mutex::new(None),
            abort: AtomicBool::new(false),
        };
        client.ensure_started()?;
        Ok(client)
    }

    /// 当前是否有活着的 worker 进程。
    pub fn is_running(&self) -> bool {
        let guard = self.lock();
        guard.is_some()
    }

    /// 最近一次成功请求的 `Instant`。
    pub fn last_used(&self) -> Option<Instant> {
        *self
            .last_used
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// 距离上次使用的时长；从未使用返回 `None`。
    pub fn idle_for(&self) -> Option<Duration> {
        self.last_used().map(|at| at.elapsed())
    }

    /// 发送一段 16k 单声道 PCM16 WAV，返回识别结果。
    ///
    /// 失败（连接错误、超时、进程退出、5xx）后 worker 会被丢弃，
    /// 下一次调用自动冷启。
    pub fn transcribe(&self, wav: &[u8], language: &str) -> Result<Transcript> {
        crate::types::validate_language(language)?;
        if wav.len() < crate::wav::HEADER_BYTES {
            return Err(SttError::InvalidInput {
                message: "audio is shorter than a WAV header".into(),
            });
        }
        if wav.len() as u64 > self.config.max_audio_bytes {
            return Err(SttError::InvalidInput {
                message: format!("audio exceeds {} bytes", self.config.max_audio_bytes),
            });
        }
        // 与 worker 侧一致：超过 max_audio_seconds 直接拒绝，省掉一次必然失败的往返。
        let max_bytes_for_seconds =
            44.0 + f64::from(self.config.max_audio_seconds.max(0.0)) * 16_000.0 * 2.0;
        if wav.len() as f64 > max_bytes_for_seconds {
            return Err(SttError::InvalidInput {
                message: format!(
                    "audio is longer than the {:.0} second limit",
                    self.config.max_audio_seconds
                ),
            });
        }
        // 串行化：同一时刻只允许一个请求。
        let mut guard = self.lock();
        if guard.is_none() {
            *guard = Some(self.start_worker()?);
        }
        // 拿到锁后清掉上一次的 abort 请求，避免它影响本次推理。
        self.abort.store(false, Ordering::SeqCst);
        let running = guard.as_mut().ok_or_else(|| SttError::Worker {
            message: "worker is not running".into(),
        })?;

        // 网络请求最长 120s；失败时必须在**释放锁之后**再回收进程，
        // 否则 `stop_process` 的等待会一直占着锁，`stop()` 只能干等到请求
        // 结束，拖慢「结束面试」。这里先拿结果、尽快释放锁，再停止进程。
        // `abort()` 置位后 post_wav 会在下一次轮询中止（无需等待 120s 超时）。
        let outcome = post_wav(running, wav, language, &self.abort);
        // 请求层面的拒绝（WAV 格式不符、语言不支持）说明 worker 本身是健康的：
        // 顺手丢弃它会让下一段音频重新冷启（加载 239MB 模型约 2.5 秒），
        // 实时提词会因此明显卡顿。其余失败一律丢弃，下次冷启。
        let discard = match &outcome {
            Ok(_) => false,
            Err(error) => !matches!(error, SttError::InvalidInput { .. }),
        };
        let dead = discard.then(|| guard.take()).flatten();
        drop(guard);
        if let Some(mut dead) = dead {
            // 任何失败（含被 abort 打断）都丢弃 worker：状态未知，下次冷启。
            stop_process(&mut dead);
        }

        match outcome {
            Ok(transcript) => {
                *self
                    .last_used
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = Some(Instant::now());
                Ok(transcript)
            }
            Err(error) => Err(error),
        }
    }

    /// 关闭 stdin 让 worker 优雅退出；1 秒后仍未退出则强杀。幂等。
    pub fn stop(&self) {
        let mut guard = self.lock();
        if let Some(mut running) = guard.take() {
            stop_process(&mut running);
        }
    }

    /// 请求中止正在进行的推理。
    ///
    /// 立即返回（不阻塞、不抢 `state` 锁）：置位后 [`WorkerClient::transcribe`]
    /// 会在下一次读取轮询时放弃当前请求并回收 worker，下次调用自动冷启。
    ///
    /// 看门狗用它打断可能已挂死（推理超过 15 秒无结果）的请求——如果不打断，
    /// `transcribe` 会持锁等到 120 秒超时，期间的看门狗判定无法落地。
    pub fn abort(&self) {
        self.abort.store(true, Ordering::SeqCst);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// 确保进程存在；不存在则冷启。
    fn ensure_started(&self) -> Result<()> {
        let mut guard = self.lock();
        if guard.is_none() {
            *guard = Some(self.start_worker()?);
        }
        Ok(())
    }

    fn start_worker(&self) -> Result<Running> {
        let token = random_token()?;
        let config_json = serde_json::to_string(&self.config)?;
        let mut command = Command::new(&self.exe);
        command
            .arg(WORKER_FLAG)
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for key in ENV_KEEP {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env(WORKER_CONFIG_ENV, config_json)
            .env(WORKER_TOKEN_ENV, &token);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        tracing::info!(exe = %self.exe.display(), "starting voice worker");
        let mut child = command.spawn().map_err(|error| SttError::Worker {
            message: format!("cannot spawn {}: {error}", self.exe.display()),
        })?;

        // stderr 后台抽干：避免管道写满卡死，并把尾部留给错误诊断。
        let stderr_tail = child.stderr.take().map(spawn_stderr_drain);
        let stdout = child.stdout.take().ok_or_else(|| SttError::Worker {
            message: "worker stdout was not piped".into(),
        })?;
        let port = match read_ready_line(stdout, &mut child, STARTUP_TIMEOUT) {
            Ok(port) => port,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(diagnose_startup_failure(attach_stderr(
                    error,
                    stderr_tail.as_ref(),
                )));
            }
        };
        tracing::info!(port, "voice worker ready");
        // 冷启也算一次「使用」：空闲回收从进程可用开始计时。
        *self
            .last_used
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Instant::now());
        Ok(Running {
            stdin: child.stdin.take(),
            child,
            port,
            auth: token,
        })
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        self.stop();
    }
}

fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| SttError::Worker {
        message: format!("cannot obtain randomness: {error}"),
    })?;
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    let mut token = String::with_capacity(43);
    let mut bits: u32 = 0;
    let mut accumulator: u32 = 0;
    for byte in bytes {
        accumulator = (accumulator << 8) | byte as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            token.push(ALPHABET[((accumulator >> bits) & 0x3f) as usize] as char);
        }
    }
    Ok(token)
}

type StderrTail = std::sync::Arc<Mutex<String>>;

fn spawn_stderr_drain(mut stderr: std::process::ChildStderr) -> StderrTail {
    let tail: StderrTail = std::sync::Arc::new(Mutex::new(String::new()));
    let sink = std::sync::Arc::clone(&tail);
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    if let Ok(mut guard) = sink.lock() {
                        guard.push_str(&text);
                        if guard.len() > 4096 {
                            let keep_from = guard.len() - 4096;
                            let boundary = guard
                                .char_indices()
                                .find(|(index, _)| *index >= keep_from)
                                .map(|(index, _)| index)
                                .unwrap_or(guard.len());
                            guard.drain(..boundary);
                        }
                    }
                }
            }
        }
    });
    tail
}

fn attach_stderr(error: SttError, tail: Option<&StderrTail>) -> SttError {
    let detail = tail
        .and_then(|tail| tail.lock().ok().map(|guard| guard.trim().to_string()))
        .unwrap_or_default();
    if detail.is_empty() {
        return error;
    }
    SttError::Worker {
        message: format!("{error} (stderr: {detail})"),
    }
}

/// 启动阶段的错误如果来自模型加载（worker 的 `engine error` / 文件缺失 /
/// recognizer 创建失败），说明磁盘上的模型虽然大小正确但内容可能已损坏——
/// 就绪判断只比大小、不重算哈希（团队确认的设计）。把它翻译成用户能照做的
/// 中文提示，而不是留下一句「worker exited during startup」。
fn diagnose_startup_failure(error: SttError) -> SttError {
    let text = error.to_string().to_ascii_lowercase();
    let looks_like_model_load = text.contains("engine error")
        || text.contains("file is missing")
        || text.contains("cannot create the sensevoice recognizer")
        || text.contains("cannot create the silero vad")
        || text.contains("cannot load");
    if looks_like_model_load {
        return SttError::Engine {
            message: format!(
                "本地语音模型加载失败，模型文件可能已损坏，请在设置页删除模型后重新下载（{error}）"
            ),
        };
    }
    error
}

/// 读 stdout 首行 `{"port":N}`。
///
/// 子进程的 stdout 是管道，没有读超时；这里用后台线程收字节 + 定时轮询进程状态，
/// 既能守住 60 秒启动上限，也能在进程提前退出时立刻失败。
fn read_ready_line(
    mut stdout: std::process::ChildStdout,
    child: &mut Child,
    timeout: Duration,
) -> Result<u16> {
    let (sender, receiver): (std::sync::mpsc::Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
    std::thread::spawn(move || {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stdout.read(&mut byte) {
                Ok(0) => break,
                Ok(_) if byte[0] == b'\n' => break,
                Ok(_) => {
                    line.push(byte[0]);
                    if line.len() > READY_LINE_LIMIT {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(line);
        // 继续抽干 stdout：worker 不该再写，但保底避免管道写满导致子进程卡死。
        let mut sink = [0u8; 4096];
        while let Ok(read) = stdout.read(&mut sink) {
            if read == 0 {
                break;
            }
        }
    });

    let deadline = Instant::now() + timeout;
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                if line.len() > READY_LINE_LIMIT {
                    return Err(SttError::Worker {
                        message: "worker readiness line is too long".into(),
                    });
                }
                return parse_ready_line(&line);
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(SttError::Worker {
                    message: "worker closed stdout before reporting its port".into(),
                });
            }
            Err(RecvTimeoutError::Timeout) => {
                if Instant::now() >= deadline {
                    return Err(SttError::Worker {
                        message: format!("worker did not report its port within {timeout:?}"),
                    });
                }
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(SttError::Worker {
                        message: format!("worker exited during startup with {status}"),
                    });
                }
            }
        }
    }
}

fn parse_ready_line(line: &[u8]) -> Result<u16> {
    let text = std::str::from_utf8(line).map_err(|_| SttError::Worker {
        message: "worker readiness line is not UTF-8".into(),
    })?;
    let value: serde_json::Value =
        serde_json::from_str(text.trim()).map_err(|_| SttError::Worker {
            message: "worker printed an invalid readiness line".into(),
        })?;
    match value.get("port").and_then(serde_json::Value::as_u64) {
        Some(port) if (1..=65_535).contains(&port) => Ok(port as u16),
        _ => Err(SttError::Worker {
            message: "worker reported an invalid port".into(),
        }),
    }
}

fn post_wav(
    running: &mut Running,
    wav: &[u8],
    language: &str,
    abort: &AtomicBool,
) -> Result<Transcript> {
    let address = format!("127.0.0.1:{}", running.port);
    let mut stream = TcpStream::connect(&address).map_err(|error| SttError::Worker {
        message: format!("cannot connect to worker on {address}: {error}"),
    })?;
    // 读超时用较短的轮询间隔，循环里再按累计时长判断 120 秒总上限；
    // 这样 abort 置位后最多等待一个轮询周期就能退出，而不是卡满 120 秒。
    let _ = stream.set_read_timeout(Some(READ_POLL));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let request = format!(
        "POST /transcribe?language={language} HTTP/1.1\r\n\
         Host: {address}\r\n\
         Authorization: Bearer {token}\r\n\
         Content-Type: audio/wav\r\n\
         Content-Length: {length}\r\n\
         Connection: close\r\n\r\n",
        token = running.auth,
        length = wav.len(),
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(wav))
        .and_then(|()| stream.flush())
        .map_err(|error| SttError::Worker {
            message: format!("cannot send audio to worker: {error}"),
        })?;

    let deadline = Instant::now() + INFERENCE_TIMEOUT;
    let mut response = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if abort.load(Ordering::SeqCst) {
            return Err(SttError::Worker {
                message: "inference aborted by the watchdog".into(),
            });
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
                if response.len() > RESPONSE_LIMIT {
                    return Err(SttError::Worker {
                        message: "worker response exceeded its size limit".into(),
                    });
                }
                // 慢速滴流也要受总超时约束：只在 WouldBlock 分支查截止时间的话，
                // 每 250ms 只吐一个字节的连接能把持锁状态拖到响应体上限。
                if Instant::now() >= deadline {
                    return Err(SttError::Worker {
                        message: format!("worker did not answer within {INFERENCE_TIMEOUT:?}"),
                    });
                }
            }
            // 轮询超时：不是错误，回到循环顶部检查 abort 与总超时。
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(SttError::Worker {
                        message: format!("worker did not answer within {INFERENCE_TIMEOUT:?}"),
                    });
                }
            }
            Err(error) => {
                return Err(SttError::Worker {
                    message: format!("cannot read worker response: {error}"),
                });
            }
        }
    }
    parse_http_response(&response)
}

/// 解析 worker 的最小 HTTP/1.1 响应（只处理本进程会遇到的形态）。
fn parse_http_response(raw: &[u8]) -> Result<Transcript> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| SttError::Worker {
            message: "worker response is missing its header terminator".into(),
        })?;
    let (head, rest) = raw.split_at(split);
    let body = &rest[4..];
    let head = std::str::from_utf8(head).map_err(|_| SttError::Worker {
        message: "worker response headers are not UTF-8".into(),
    })?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| SttError::Worker {
            message: "worker response has an invalid status line".into(),
        })?;

    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = value.parse::<usize>().ok();
        } else if name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
            chunked = true;
        }
    }

    let body = if chunked {
        decode_chunked(body)?
    } else {
        match content_length {
            Some(length) if length <= body.len() => body[..length].to_vec(),
            Some(length) => {
                return Err(SttError::Worker {
                    message: format!(
                        "worker response declared {length} bytes but sent {}",
                        body.len()
                    ),
                });
            }
            None => body.to_vec(),
        }
    };

    if status == 200 {
        serde_json::from_slice::<Transcript>(&body).map_err(|_| SttError::Worker {
            message: "worker returned an unexpected response body".into(),
        })
    } else {
        let error: WorkerErrorBody = serde_json::from_slice(&body)
            .unwrap_or_else(|_| WorkerErrorBody::new(format!("HTTP {status}")));
        if error.code.as_deref() == Some("invalid-input") {
            Err(SttError::InvalidInput {
                message: error.error,
            })
        } else if status == 401 {
            Err(SttError::Worker {
                message: "worker rejected the token".into(),
            })
        } else {
            Err(SttError::Worker {
                message: format!("worker failed (HTTP {status}): {}", error.error),
            })
        }
    }
}

fn decode_chunked(mut raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let Some(position) = raw.windows(2).position(|window| window == b"\r\n") else {
            return Err(SttError::Worker {
                message: "malformed chunked worker response".into(),
            });
        };
        let (size_line, rest) = raw.split_at(position);
        let size_text = std::str::from_utf8(size_line).map_err(|_| SttError::Worker {
            message: "malformed chunk size".into(),
        })?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| SttError::Worker {
                message: "malformed chunk size".into(),
            })?;
        raw = &rest[2..];
        if size == 0 {
            return Ok(out);
        }
        // 畸形长度不能让分发线程 panic：用 checked_add 并检查实际剩余长度。
        let Some(end) = size.checked_add(2) else {
            return Err(SttError::Worker {
                message: "malformed chunk size".into(),
            });
        };
        if raw.len() < end {
            return Err(SttError::Worker {
                message: "truncated chunk in worker response".into(),
            });
        }
        out.extend_from_slice(&raw[..size]);
        raw = &raw[end..];
    }
}

fn stop_process(running: &mut Running) {
    // 先关 stdin：worker 的退出条件之一（Windows 上没有可用的 SIGTERM）。
    drop(running.stdin.take());
    let deadline = Instant::now() + STOP_GRACE;
    loop {
        match running.child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(_) => break,
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = running.child.kill();
    let _ = running.child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_success_response() {
        let body = r#"{"text":"你好","audioSeconds":1.5,"inferenceSeconds":0.2}"#;
        let raw = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let transcript = parse_http_response(raw.as_bytes()).unwrap();
        assert_eq!(transcript.text, "你好");
        assert_eq!(transcript.audio_seconds, 1.5);
    }

    #[test]
    fn maps_invalid_input_and_other_errors() {
        let body = r#"{"error":"bad wav","code":"invalid-input"}"#;
        let raw = format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        assert!(matches!(
            parse_http_response(raw.as_bytes()),
            Err(SttError::InvalidInput { .. })
        ));

        let raw = "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 2\r\n\r\n{}";
        assert!(matches!(
            parse_http_response(raw.as_bytes()),
            Err(SttError::Worker { .. })
        ));
    }

    #[test]
    fn decodes_chunked_bodies() {
        let raw = b"4\r\nabcd\r\n3\r\nefg\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(raw).unwrap(), b"abcdefg");
    }

    #[test]
    fn token_is_url_safe_and_long_enough() {
        let token = random_token().unwrap();
        // 32 字节按 6bit 打包得到 42 个字符（末 4 位丢弃），满足 32+ 位要求。
        assert_eq!(token.len(), 42);
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn ready_line_parser_rejects_garbage() {
        assert_eq!(parse_ready_line(b"{\"port\":8123}").unwrap(), 8123);
        assert!(parse_ready_line(b"{\"port\":0}").is_err());
        assert!(parse_ready_line(b"{\"port\":70000}").is_err());
        assert!(parse_ready_line(b"not json").is_err());
    }
}
