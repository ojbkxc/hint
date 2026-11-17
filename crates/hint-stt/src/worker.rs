//! sidecar worker 的推理与 loopback HTTP 服务（`engine` feature）。
//!
//! 调用链：
//! ```text
//! 读 HINT_VOICE_WORKER_CONFIG / HINT_VOICE_WORKER_TOKEN → 立即删除环境变量
//!   → 构建 SenseVoice recognizer + Silero VAD
//!   → 监听 127.0.0.1:0
//!   → stdout 首行 {"port":N}（父进程据此判定模型已加载完成）
//!   → POST /transcribe?language=auto（Bearer token + 原始 WAV 体）
//!   → stdin EOF 或 SIGTERM 退出
//! ```
//!
//! VAD 参数固定为 cc-haha 的实测调参：window 512、threshold 0.5、
//! min_speech 0.25s、min_silence 0.5s、max_speech 30s。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};
use subtle::ConstantTimeEq;

use crate::error::{Result, SttError};
use crate::types::{
    Transcript, WORKER_CONFIG_ENV, WORKER_TOKEN_ENV, WorkerConfig, WorkerErrorBody,
    validate_language,
};
use crate::wav;

/// VAD 窗口：512 采样点（32ms @16k）。
pub const VAD_WINDOW: usize = 512;
/// HTTP 请求头读取上限。
const HEADER_LIMIT: usize = 16 * 1024;
/// token 长度下限。
const TOKEN_MIN_LEN: usize = 32;

/// SenseVoice + Silero VAD 的组合。
///
/// 两者都是 `Send + Sync`，推理是同步的：请求处理由外层 `Mutex` 串行化。
struct Engine {
    recognizer: OfflineRecognizer,
    vad: VoiceActivityDetector,
    /// 构造好的模型配置模板：切换语言时用它重建 recognizer。
    recognizer_config: OfflineRecognizerConfig,
    /// 当前 recognizer 对应的语言提示。切换语言时按模板重建 recognizer。
    language: String,
}

impl Engine {
    fn load(config: &WorkerConfig) -> Result<Self> {
        for (label, path) in [
            ("model", &config.model_path),
            ("tokens", &config.tokens_path),
            ("vad", &config.vad_path),
        ] {
            if !path.is_file() {
                return Err(SttError::Engine {
                    message: format!("{label} file is missing: {}", path.display()),
                });
            }
        }

        // SenseVoice 的语言提示只在构造时生效（2.0.1 没有 `set_config`），
        // 所以模板保留一份配置，切语言时按模板重建 recognizable。
        let mut recognizer_config = OfflineRecognizerConfig::default();
        recognizer_config.feat_config.sample_rate = 16_000;
        recognizer_config.feat_config.feature_dim = 80;
        recognizer_config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(config.model_path.display().to_string()),
            language: Some("auto".to_string()),
            use_itn: true,
        };
        recognizer_config.model_config.tokens = Some(config.tokens_path.display().to_string());
        recognizer_config.model_config.num_threads = config.threads;
        recognizer_config.model_config.provider = Some("cpu".to_string());
        recognizer_config.model_config.debug = false;

        let recognizer = build_recognizer(&recognizer_config, "auto")?;

        let vad_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(config.vad_path.display().to_string()),
                threshold: config.vad_threshold,
                min_silence_duration: config.min_silence_seconds,
                min_speech_duration: config.min_speech_seconds,
                window_size: VAD_WINDOW as i32,
                max_speech_duration: config.max_speech_seconds,
            },
            sample_rate: 16_000,
            num_threads: config.threads,
            provider: Some("cpu".to_string()),
            debug: false,
            ..VadModelConfig::default()
        };
        let vad = VoiceActivityDetector::create(&vad_config, config.vad_buffer_seconds())
            .ok_or_else(|| SttError::Engine {
                message: "cannot create the Silero VAD".into(),
            })?;

        tracing::info!(
            threads = config.threads,
            max_speech = config.max_speech_seconds,
            "sensevoice engine loaded"
        );
        Ok(Self {
            recognizer,
            vad,
            recognizer_config,
            language: "auto".to_string(),
        })
    }

    /// 对一个完整 WAV 做 VAD 分段 + 逐段识别。
    fn transcribe(&mut self, wav_bytes: &[u8], language: &str) -> Result<Transcript> {
        let pcm = wav::read_pcm16_mono_16k(wav_bytes)?;
        let started = Instant::now();

        // 与 cc-haha 相同：只有 SenseVoice 吃语言提示；切换时重建 recognizer。
        // 重建只发生在语言变化的那一次请求上，加载 239MB 模型约 1–3s；
        // 上层（设置页）通常固定一种语言，日常请求不会触发。
        if language != self.language {
            self.recognizer = build_recognizer(&self.recognizer_config, language)?;
            self.language = language.to_string();
        }

        self.vad.reset();
        let mut texts: Vec<String> = Vec::new();
        for chunk in pcm.samples.chunks(VAD_WINDOW) {
            self.vad.accept_waveform(chunk);
            self.drain(&mut texts)?;
        }
        self.vad.flush();
        self.drain(&mut texts)?;

        let text = texts
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Transcript {
            text: text.trim().to_string(),
            audio_seconds: pcm.seconds(),
            inference_seconds: started.elapsed().as_secs_f32(),
        })
    }

    /// 取出所有已完成的 VAD 段并逐段解码。
    fn drain(&mut self, texts: &mut Vec<String>) -> Result<()> {
        while !self.vad.is_empty() {
            let Some(segment) = self.vad.front() else {
                break;
            };
            // 拷贝出原生内存，避免 pop 后指针失效。
            let samples = segment.samples().to_vec();
            drop(segment);
            self.vad.pop();
            if samples.is_empty() {
                continue;
            }
            let stream = self.recognizer.create_stream();
            stream.accept_waveform(16_000, &samples);
            self.recognizer.decode(&stream);
            texts.push(
                stream
                    .get_result()
                    .map(|result| result.text.trim().to_string())
                    .unwrap_or_default(),
            );
        }
        Ok(())
    }
}

/// 按语言提示构建一个 recognizer。`auto` 保持模板原样。
fn build_recognizer(
    template: &OfflineRecognizerConfig,
    language: &str,
) -> Result<OfflineRecognizer> {
    let mut config = template.clone();
    config.model_config.sense_voice.language = Some(language.to_string());
    OfflineRecognizer::create(&config).ok_or_else(|| SttError::Engine {
        message: format!("cannot create the SenseVoice recognizer (language={language})"),
    })
}

/// 常量化状态：HTTP 请求处理用。
struct ServerState {
    engine: std::sync::Mutex<Engine>,
    /// 期望的 `Authorization: Bearer <token>` 原文。
    expected_auth: Vec<u8>,
    config: WorkerConfig,
}

/// 从环境变量读取配置，**读完立即删除**。
fn read_env_config() -> Result<(WorkerConfig, String)> {
    let raw = std::env::var(WORKER_CONFIG_ENV).ok();
    let token = std::env::var(WORKER_TOKEN_ENV).ok();
    // SAFETY: `remove_var` 在 Rust 2024 是 unsafe：多线程并发读写环境变量是 UB。
    // worker 在创建任何线程之前调用本函数，且此后再也不读环境变量。
    unsafe {
        std::env::remove_var(WORKER_CONFIG_ENV);
        std::env::remove_var(WORKER_TOKEN_ENV);
    }
    let (Some(raw), Some(token)) = (raw, token) else {
        return Err(SttError::Config {
            message: "voice worker was started without its configuration".into(),
        });
    };
    if token.len() < TOKEN_MIN_LEN
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(SttError::Config {
            message: "voice worker token is missing or malformed".into(),
        });
    }
    let config: WorkerConfig = serde_json::from_str(&raw).map_err(|error| SttError::Config {
        message: format!("cannot parse the worker configuration: {error}"),
    })?;
    config.validate()?;
    Ok((config, token))
}

/// worker 入口。只有父进程传 `--voice-worker` 时调用。
///
/// 正常返回意味着父进程关闭了 stdin；错误带上下文便于父进程贴进 stderr 诊断。
pub fn run_worker_from_env() -> Result<()> {
    let (config, token) = read_env_config()?;
    let engine = Engine::load(&config)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| SttError::Worker {
        message: format!("cannot bind the loopback listener: {error}"),
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| SttError::Worker {
            message: format!("cannot read the listener address: {error}"),
        })?
        .port();

    let state = Arc::new(ServerState {
        engine: std::sync::Mutex::new(engine),
        expected_auth: format!("Bearer {token}").into_bytes(),
        config,
    });
    let stop = Arc::new(AtomicBool::new(false));

    // stdin 监听：父进程关 stdin（Windows 上的优雅停止）时置位。
    {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buffer = [0u8; 64];
            loop {
                match stdin.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            stop.store(true, Ordering::SeqCst);
        });
    }

    install_termination_handler(Arc::clone(&stop));

    listener
        .set_nonblocking(true)
        .map_err(|error| SttError::Worker {
            message: format!("cannot set the listener non-blocking: {error}"),
        })?;

    // 就绪行必须在开始服务前输出：父进程只据此判定启动成功。
    {
        let line = format!("{{\"port\":{port}}}\n");
        let mut stdout = std::io::stdout();
        stdout
            .write_all(line.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| SttError::Worker {
                message: format!("cannot write the readiness line: {error}"),
            })?;
    }
    tracing::info!(port, "voice worker listening");

    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                // 只监听 127.0.0.1 已经足够；这层是纵深防御。
                if !peer.ip().is_loopback() {
                    continue;
                }
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &state) {
                        tracing::debug!(%error, "worker request failed");
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                return Err(SttError::Worker {
                    message: format!("accept failed: {error}"),
                });
            }
        }
    }
    tracing::info!("voice worker stopping");
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP 处理
// ---------------------------------------------------------------------------

fn handle_connection(stream: TcpStream, state: &ServerState) -> Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(120)));
    let write_handle = stream.try_clone().map_err(|error| SttError::Worker {
        message: format!("cannot clone the socket: {error}"),
    })?;
    let mut reader = BufReader::new(stream);

    let (request_line, headers) = read_request_head(&mut reader)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    // 1. 认证：常量时间比较；长度不等直接 401。
    let supplied = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_bytes())
        .unwrap_or_default();
    let expected = &state.expected_auth;
    if supplied.len() != expected.len() || !bool::from(supplied.ct_eq(expected.as_slice())) {
        return write_json(&write_handle, 401, &WorkerErrorBody::new("Unauthorized"));
    }

    // 2. 路由。
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    };
    if method != "POST" || path != "/transcribe" {
        return write_json(
            &write_handle,
            404,
            &WorkerErrorBody::new("Unknown endpoint"),
        );
    }

    // 3. Content-Length 校验：46 ≤ len ≤ maxAudioBytes。
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok());
    let Some(length) = length else {
        return write_json(
            &write_handle,
            413,
            &WorkerErrorBody::invalid("audio length is missing"),
        );
    };
    if length < wav::HEADER_BYTES as u64 || length > state.config.max_audio_bytes {
        return write_json(
            &write_handle,
            413,
            &WorkerErrorBody::invalid("audio size is outside the accepted range"),
        );
    }

    // 4. 读请求体。
    let mut body = vec![0u8; length as usize];
    reader
        .read_exact(&mut body)
        .map_err(|error| SttError::Worker {
            message: format!("cannot read the request body: {error}"),
        })?;

    // 5. 语言参数。
    let language = match parse_language(query) {
        Some(language) if validate_language(&language).is_ok() => language,
        Some(_) => {
            return write_json(
                &write_handle,
                400,
                &WorkerErrorBody::invalid("unsupported speech language"),
            );
        }
        None => "auto".to_string(),
    };

    // 6. 推理（串行；native 推理同步）。
    let result = {
        let mut engine = state
            .engine
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        engine.transcribe(&body, &language)
    };
    match result {
        Ok(transcript) => write_json(&write_handle, 200, &transcript),
        Err(SttError::InvalidInput { message }) => {
            write_json(&write_handle, 400, &WorkerErrorBody::invalid(message))
        }
        Err(error) => write_json(&write_handle, 500, &WorkerErrorBody::new(error.to_string())),
    }
}

fn read_request_head(reader: &mut BufReader<TcpStream>) -> Result<(String, Vec<(String, String)>)> {
    let mut request_line = String::new();
    let mut consumed = 0usize;
    reader
        .read_line(&mut request_line)
        .map_err(|error| SttError::Worker {
            message: format!("cannot read the request line: {error}"),
        })?;
    consumed += request_line.len();
    if request_line.trim().is_empty() {
        return Err(SttError::Worker {
            message: "connection closed before a request arrived".into(),
        });
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| SttError::Worker {
                message: format!("cannot read request headers: {error}"),
            })?;
        consumed += read;
        if consumed > HEADER_LIMIT {
            return Err(SttError::Worker {
                message: "request headers are too large".into(),
            });
        }
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    Ok((request_line.trim_end().to_string(), headers))
}

fn parse_language(query: Option<&str>) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("language=") {
            return Some(percent_decode(value));
        }
    }
    None
}

/// 仅处理 `%xx` 与 `+`；语言码只含 ASCII 字母，足够。
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 写一个最小 JSON 响应（`Connection: close`，定长 body，无需 chunked）。
fn write_json(stream: &TcpStream, status: u16, body: &impl serde::Serialize) -> Result<()> {
    let payload = serde_json::to_vec(body)?;
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         content-type: application/json\r\n\
         content-length: {length}\r\n\
         connection: close\r\n\r\n",
        reason = status_reason(status),
        length = payload.len(),
    );
    let mut writer = stream.try_clone().map_err(|error| SttError::Worker {
        message: format!("cannot clone the socket for writing: {error}"),
    })?;
    writer
        .write_all(head.as_bytes())
        .and_then(|()| writer.write_all(&payload))
        .and_then(|()| writer.flush())
        .map_err(|error| SttError::Worker {
            message: format!("cannot write the response: {error}"),
        })
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// 退出信号
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn install_termination_handler(stop: Arc<AtomicBool>) {
    // Windows 没有可用的 SIGTERM：父进程关闭 stdin 后本进程会立刻看到 EOF。
    // 任务管理器/TerminateProcess 是硬终止，无需处理。保留函数与 Unix 分支对称。
    let _ = stop;
}

#[cfg(unix)]
fn install_termination_handler(stop: Arc<AtomicBool>) {
    use std::sync::OnceLock;
    static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    let _ = STOP.set(stop);

    extern "C" fn on_signal(_signal: libc::c_int) {
        if let Some(stop) = STOP.get() {
            stop.store(true, Ordering::SeqCst);
        }
    }
    // SAFETY: 处理器只写一个静态 AtomicBool，属于 async-signal-safe 操作。
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as libc::sighandler_t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_query_parsing() {
        assert_eq!(parse_language(Some("language=zh")), Some("zh".into()));
        assert_eq!(parse_language(Some("language=%7A%68")), Some("zh".into()));
        assert_eq!(
            parse_language(Some("foo=1&language=auto")),
            Some("auto".into())
        );
        assert_eq!(parse_language(Some("foo=1")), None);
        assert_eq!(parse_language(None), None);
    }

    #[test]
    fn status_reasons_are_stable() {
        assert_eq!(status_reason(200), "OK");
        assert_eq!(status_reason(413), "Payload Too Large");
    }
}
