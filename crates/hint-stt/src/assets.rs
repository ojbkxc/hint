//! 编译期固化的语音资产清单。
//!
//! 所有地址、字节数与 sha256 均已在发布前实测核验（见 `voice-asset-manifest.md`，
//! 2026-11-17）。清单写死在二进制里，不从远端配置读取：换模型 = 改代码重新发版。
//!
//! 单位约定：`bytes` 为十进制字节数，`sha256` 为 64 字符小写 hex。

/// 模型标识，供上层（设置页、事件）使用。
pub const MODEL_ID: &str = "sensevoice-local";

/// 模型目录名。`ModelManager` 的全部文件都落在 `<data_root>/sensevoice/` 下。
pub const DIR_NAME: &str = "sensevoice";

/// SenseVoice 仓库与固定的完整 40 位 commit。
pub const HF_REPO: &str = "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";
pub const HF_REVISION: &str = "2365baeacb507f821a0c8120fcee3d484dba7a07";

/// Silero VAD 仓库与 revision。用 v4 这份（`silero_vad.onnx`），不要换 v5：
/// sherpa-onnx 的 `Vad` 配置按 v4 的输入形状调过。
pub const VAD_REPO: &str = "csukuangfj/vad";
pub const VAD_REVISION: &str = "fba88cd2e921609e7675c3aaf51e0b9b295da4bc";

/// 下载源基址，按优先级排列：官方 → hf-mirror。任一失败继续下一个。
///
/// 实测（2026-11-17）：`hf-mirror.com` 可能 308 跳回 huggingface.co，
/// 所以镜像只是兜底，不能因为配了镜像就跳过官方。
pub const ORIGINS: &[Origin] = &[
    Origin {
        label: "official",
        base: "https://huggingface.co",
    },
    Origin {
        label: "mirror",
        base: "https://hf-mirror.com",
    },
];

/// 一个下载源。
#[derive(Debug, Clone, Copy)]
pub struct Origin {
    /// 进度/日志里的短名。
    pub label: &'static str,
    /// 不带尾斜杠的 origin。
    pub base: &'static str,
}

/// 一个不可变的远端文件。所有值都在发布前核验过。
#[derive(Debug, Clone, Copy)]
pub struct RemoteFile {
    /// 仓库内文件名，同时用于进度展示。
    pub name: &'static str,
    /// 落盘路径（相对模型目录，正斜杠分隔）。
    pub target: &'static str,
    /// 期望字节数。
    pub bytes: u64,
    /// 小写 hex sha256。
    pub sha256: &'static str,
    /// 所属 HF 仓库。
    pub repo: &'static str,
    /// 固定 revision（完整 40 位 commit）。
    pub revision: &'static str,
}

/// SenseVoice Small INT8 + tokens。顺序即安装顺序（模型 → tokens → VAD）。
pub const MODEL_FILES: &[RemoteFile] = &[
    RemoteFile {
        name: "model.int8.onnx",
        target: "models/model.int8.onnx",
        bytes: 239_233_841,
        sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
        repo: HF_REPO,
        revision: HF_REVISION,
    },
    RemoteFile {
        name: "tokens.txt",
        target: "models/tokens.txt",
        bytes: 315_894,
        sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
        repo: HF_REPO,
        revision: HF_REVISION,
    },
];

/// Silero VAD（v4）。总是最后一个安装。
pub const VAD_FILE: RemoteFile = RemoteFile {
    name: "silero_vad.onnx",
    target: "models/silero_vad.onnx",
    bytes: 1_807_522,
    sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
    repo: VAD_REPO,
    revision: VAD_REVISION,
};

/// 三个文件合计字节数：241,357,257。
pub const TOTAL_BYTES: u64 = 239_233_841 + 315_894 + 1_807_522;

/// 首次下载前预检的磁盘配额（含 `.part` 临时文件与解包暂存），约 300MB。
pub const REQUIRED_DISK_BYTES: u64 = 300 * 1024 * 1024;

/// 清单里的全部文件，顺序即安装顺序。
pub const ALL_FILES: [&RemoteFile; 3] = [&MODEL_FILES[0], &MODEL_FILES[1], &VAD_FILE];

/// SenseVoice 模型在模型目录中的相对路径（正斜杠）。
pub const MODEL_PATH: &str = "models/model.int8.onnx";
/// tokens 的模型相对路径。
pub const TOKENS_PATH: &str = "models/tokens.txt";
/// VAD 的模型相对路径。
pub const VAD_PATH: &str = "models/silero_vad.onnx";

/// 按「源基址 + 仓库 + revision + 文件名」拼出完整 resolve URL。
///
/// 输出顺序与 [`ORIGINS`] 一致；调用方按顺序尝试即可。
pub fn resolve_urls(file: &RemoteFile) -> Vec<String> {
    ORIGINS
        .iter()
        .map(|origin| resolve_url(origin, file))
        .collect()
}

/// 单个源上的 URL。
pub fn resolve_url(origin: &Origin, file: &RemoteFile) -> String {
    format!(
        "{}/{}/resolve/{}/{}",
        origin.base, file.repo, file.revision, file.name
    )
}

/// 把清单里的 `sha256` 解析成 32 字节数组。长度或字符非法时返回 `None`。
pub fn decode_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(out)
}

/// 把 32 字节摘要编码成小写 hex。
pub fn encode_sha256(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_values_match_the_verified_sheet() {
        assert_eq!(MODEL_FILES.len(), 2);
        assert_eq!(MODEL_FILES[0].bytes, 239_233_841);
        assert_eq!(MODEL_FILES[1].bytes, 315_894);
        assert_eq!(VAD_FILE.bytes, 1_807_522);
        assert_eq!(TOTAL_BYTES, 241_357_257);
        assert_eq!(HF_REVISION.len(), 40);
        assert_eq!(VAD_REVISION.len(), 40);
    }

    #[test]
    fn urls_follow_origin_order() {
        let urls = resolve_urls(&MODEL_FILES[0]);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].starts_with("https://huggingface.co/csukuangfj/"));
        assert!(
            urls[0].ends_with("/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/model.int8.onnx")
        );
        assert!(urls[1].starts_with("https://hf-mirror.com/csukuangfj/"));
        let vad = resolve_urls(&VAD_FILE);
        assert!(vad[0].contains(
            "/csukuangfj/vad/resolve/fba88cd2e921609e7675c3aaf51e0b9b295da4bc/silero_vad.onnx"
        ));
    }

    #[test]
    fn sha256_hex_roundtrip() {
        let expected = MODEL_FILES[0].sha256;
        let digest = decode_sha256(expected).expect("valid hex");
        assert_eq!(encode_sha256(&digest), expected);
        assert!(decode_sha256("xyz").is_none());
        assert!(decode_sha256(&expected[..63]).is_none());
        // 大小写不敏感
        assert_eq!(
            encode_sha256(&decode_sha256(&expected.to_uppercase()).unwrap()),
            expected
        );
    }

    #[test]
    fn target_paths_are_relative_and_forward_slashed() {
        for file in ALL_FILES {
            assert!(!file.target.starts_with('/'));
            assert!(!file.target.contains('\\'));
            assert!(file.target.starts_with("models/"));
        }
        assert_eq!(MODEL_FILES[0].target, MODEL_PATH);
        assert_eq!(MODEL_FILES[1].target, TOKENS_PATH);
        assert_eq!(VAD_FILE.target, VAD_PATH);
    }
}
