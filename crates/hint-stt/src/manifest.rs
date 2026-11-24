//! `verified.json` 读写与就绪判断。
//!
//! 就绪检查**不重算 hash**：只比对「清单记录存在 + 磁盘文件存在 + 大小相等」。
//! 239MB 每次启动算一遍要几秒，用户会以为程序卡死。mtime 有意不参与判断——
//! 备份、复制、同步工具会重写 mtime 但内容完好。
//!
//! 只有三种情况才重算 sha256：
//! 1. 首次下载完成时（下载器边下边算，天然覆盖）；
//! 2. 用户手动点「校验文件」（上层调用 [`verify_files`]）；
//! 3. 文件大小与记录不符（视为未安装，触发重新下载）。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets::{self, RemoteFile, encode_sha256};
use crate::error::{Result, SttError};

/// 清单版本号，读到其它值视为损坏清单。
pub const MANIFEST_VERSION: u32 = 1;

/// `verified.json` 里的一条记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedFile {
    /// 相对模型目录的正斜杠路径，例如 `models/model.int8.onnx`。
    pub name: String,
    /// 验证时的字节数。
    pub bytes: u64,
    /// 小写 hex sha256。
    pub sha256: String,
}

/// `verified.json` 的完整结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub files: Vec<VerifiedFile>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            files: Vec::new(),
        }
    }
}

/// 模型目录布局。
#[derive(Debug, Clone)]
pub struct Layout {
    /// `<data_root>/sensevoice`
    pub base: PathBuf,
    /// `<base>/models`
    pub models: PathBuf,
    /// `<base>/downloads`：`.part` 临时文件
    pub downloads: PathBuf,
    /// `<base>/staging`：预留
    pub staging: PathBuf,
    /// `<base>/verified.json`
    pub manifest: PathBuf,
}

impl Layout {
    /// 按数据根构造布局。
    pub fn new(data_root: &Path) -> Self {
        let base = data_root.join(assets::DIR_NAME);
        Self {
            models: base.join("models"),
            downloads: base.join("downloads"),
            staging: base.join("staging"),
            manifest: base.join("verified.json"),
            base,
        }
    }

    /// 清单相对路径对应的绝对路径。
    pub fn file_path(&self, target: &str) -> PathBuf {
        // target 用正斜杠分隔，逐段 join 以便 Windows 正确生成反斜杠。
        let mut path = self.base.clone();
        for segment in target.split('/') {
            path.push(segment);
        }
        path
    }

    /// `.part` 路径：放在 `downloads/` 下，用文件名派生，保证前后两次下载同名。
    pub fn part_path(&self, file: &RemoteFile) -> PathBuf {
        self.downloads.join(format!("{}.part", file.name))
    }

    /// 建立 `models` / `downloads` / `staging` 目录。
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [&self.base, &self.models, &self.downloads, &self.staging] {
            fs::create_dir_all(dir).map_err(|error| {
                SttError::storage(format!("cannot create {}: {error}", dir.display()))
            })?;
        }
        Ok(())
    }
}

/// 读取清单。文件缺失或 JSON 损坏返回空清单（等价于「未安装」）。
///
/// **不再按 `version` 字段硬性拒绝**：原先要求 `version == MANIFEST_VERSION`，
/// 这意味着以后任何一次 bump（哪怕只是加一个字段）都会让所有用户的清单被
/// 判为无效 → `all_installed` 返回 false → 重新下载 241 MB 模型。
/// 兼容性由 serde 本身判断：`files` 能解析出来就说明结构兼容；具体文件是否
/// 有效由 [`is_installed`] 的 sha256 + 大小二次校验兜底。
///
/// 读进来的 `version` 统一改写成当前常量，避免下游看到旧值。
pub fn read_manifest(path: &Path) -> Manifest {
    let Ok(text) = fs::read_to_string(path) else {
        return Manifest::default();
    };
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        return Manifest::default();
    };
    Manifest {
        version: MANIFEST_VERSION,
        files: manifest
            .files
            .into_iter()
            .filter(|file| !file.name.is_empty() && file.bytes > 0 && !file.sha256.is_empty())
            .collect(),
    }
}

/// 原子写清单：先写 `.tmp` 再 rename，避免断电留下半截 JSON。
pub fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            SttError::storage(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let text = serde_json::to_string_pretty(manifest)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, format!("{text}\n"))
        .map_err(|error| SttError::storage(format!("cannot write {}: {error}", temp.display())))?;
    // Windows 上 rename 覆盖已有文件是允许的（ReplaceFile/MoveFileEx 语义）。
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        SttError::storage(format!("cannot replace {}: {error}", path.display()))
    })?;
    Ok(())
}

/// 按名字合并记录：新记录覆盖同名的旧记录，其它条目保留。
pub fn merge_records(existing: &Manifest, entries: &[VerifiedFile]) -> Manifest {
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    let mut files: Vec<VerifiedFile> = existing
        .files
        .iter()
        .filter(|file| !names.contains(&file.name.as_str()))
        .cloned()
        .collect();
    files.extend_from_slice(entries);
    Manifest {
        version: MANIFEST_VERSION,
        files,
    }
}

/// 把一条已核验的文件信息追加/更新进清单并落盘。
pub fn record_verified(manifest_path: &Path, entry: VerifiedFile) -> Result<()> {
    let existing = read_manifest(manifest_path);
    let merged = merge_records(&existing, std::slice::from_ref(&entry));
    write_manifest(manifest_path, &merged)
}

/// 删除清单文件。
pub fn forget_manifest(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SttError::storage(format!(
            "cannot remove {}: {error}",
            path.display()
        ))),
    }
}

/// 一条记录是否仍然描述磁盘上的文件（存在 + 大小相等）。不重算 hash。
pub fn matches_record(entry: Option<&VerifiedFile>, file_path: &Path, expected_bytes: u64) -> bool {
    let Some(entry) = entry else {
        return false;
    };
    if entry.bytes != expected_bytes {
        return false;
    }
    match fs::metadata(file_path) {
        Ok(meta) => meta.is_file() && meta.len() == entry.bytes,
        Err(_) => false,
    }
}

/// 清单里某个 `target` 的记录。
pub fn find_record<'a>(manifest: &'a Manifest, target: &str) -> Option<&'a VerifiedFile> {
    manifest.files.iter().find(|file| file.name == target)
}

/// `target` 是否已经安装就绪（快路径）。
pub fn is_installed(layout: &Layout, manifest: &Manifest, file: &RemoteFile) -> bool {
    let entry = find_record(manifest, file.target);
    // 记录必须指向清单**当前**期望的摘要：只比大小的快路径会让升级换模型
    // （或同大小但内容不同的旧文件）被判成已安装，worker 随后加载坏模型。
    if !entry.is_some_and(|entry| entry.sha256.eq_ignore_ascii_case(file.sha256)) {
        return false;
    }
    matches_record(entry, &layout.file_path(file.target), file.bytes)
}

/// 三个文件是否全部就绪。
pub fn all_installed(layout: &Layout, manifest: &Manifest) -> bool {
    assets::ALL_FILES
        .iter()
        .all(|file| is_installed(layout, manifest, file))
}

/// 逐个文件重算 sha256 做完整校验（用户手动触发）。
///
/// 任一文件缺失、大小不符或 sha256 不符都返回 `Err(Integrity)`；
/// 上层随后可以删除 `verified.json` 强制重新下载。
pub fn verify_files(layout: &Layout) -> Result<()> {
    for file in assets::ALL_FILES {
        let path = layout.file_path(file.target);
        let meta = fs::metadata(&path).map_err(|_| SttError::Integrity {
            message: format!("{} is missing", file.name),
        })?;
        if meta.len() != file.bytes {
            return Err(SttError::Integrity {
                message: format!(
                    "{} has {} bytes, expected {}",
                    file.name,
                    meta.len(),
                    file.bytes
                ),
            });
        }
        let actual = hash_file(&path)?;
        if !actual.eq_ignore_ascii_case(file.sha256) {
            return Err(SttError::Integrity {
                message: format!("{} sha256 mismatch", file.name),
            });
        }
    }
    Ok(())
}

/// 流式计算文件的 sha256。
pub fn hash_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = fs::File::open(path)
        .map_err(|error| SttError::storage(format!("cannot open {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            SttError::storage(format!("cannot read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    Ok(encode_sha256(&array))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hint-stt-manifest-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn manifest_roundtrip_and_defaults() {
        let root = temp_root("roundtrip");
        let layout = Layout::new(&root);
        layout.ensure_dirs().unwrap();
        assert_eq!(read_manifest(&layout.manifest), Manifest::default());

        let entry = VerifiedFile {
            name: assets::MODEL_PATH.to_string(),
            bytes: assets::MODEL_FILES[0].bytes,
            sha256: assets::MODEL_FILES[0].sha256.to_string(),
        };
        record_verified(&layout.manifest, entry.clone()).unwrap();
        let manifest = read_manifest(&layout.manifest);
        assert_eq!(manifest.version, MANIFEST_VERSION);
        assert_eq!(manifest.files, vec![entry.clone()]);

        // 同名记录覆盖，其它条目保留。
        let second = VerifiedFile {
            name: assets::TOKENS_PATH.to_string(),
            bytes: assets::MODEL_FILES[1].bytes,
            sha256: assets::MODEL_FILES[1].sha256.to_string(),
        };
        record_verified(&layout.manifest, second.clone()).unwrap();
        record_verified(&layout.manifest, entry.clone()).unwrap();
        let updated = read_manifest(&layout.manifest);
        assert_eq!(updated.files.len(), 2);
        assert_eq!(updated.files[0].name, second.name);
        assert_eq!(updated.files[1].name, entry.name);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupted_manifest_reads_as_empty() {
        let root = temp_root("corrupt");
        let layout = Layout::new(&root);
        layout.ensure_dirs().unwrap();
        fs::write(&layout.manifest, "{not json").unwrap();
        assert_eq!(read_manifest(&layout.manifest), Manifest::default());
        // 非 JSON / 缺 files 的清单回落空清单。
        // 注意：这里**不再**用「version 不等于当前常量」来构造退化场景——
        // 版本门控已移除（见 `stale_manifest_version_still_counts_as_installed`），
        // 现在只有结构真的解析不出来才算无效。
        fs::write(&layout.manifest, r#"{"version":99,"files":[]}"#).unwrap();
        assert_eq!(
            read_manifest(&layout.manifest),
            Manifest::default(),
            "空 files 等价于未安装"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn readiness_needs_record_and_matching_size() {
        let root = temp_root("ready");
        let layout = Layout::new(&root);
        layout.ensure_dirs().unwrap();
        let file = assets::MODEL_FILES[1];
        let path = layout.file_path(file.target);
        fs::write(&path, vec![0u8; 10]).unwrap();
        let entry = VerifiedFile {
            name: file.target.to_string(),
            bytes: 10,
            sha256: file.sha256.to_string(),
        };
        // 记录的字节数和清单不符 → 未就绪
        assert!(!matches_record(Some(&entry), &path, file.bytes));
        // 记录缺失 → 未就绪
        assert!(!matches_record(None, &path, 10));
        // 大小一致 → 就绪
        assert!(matches_record(Some(&entry), &path, 10));
        // 文件被截断 → 未就绪
        fs::write(&path, vec![0u8; 9]).unwrap();
        assert!(!matches_record(Some(&entry), &path, 10));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn readiness_requires_the_manifest_digest() {
        let root = temp_root("digest");
        let layout = Layout::new(&root);
        layout.ensure_dirs().unwrap();
        let file = assets::MODEL_FILES[1];
        let path = layout.file_path(file.target);
        fs::write(&path, vec![0u8; file.bytes as usize]).unwrap();

        // 大小对得上、但记录里的摘要不是清单当前的摘要（旧版本清单 /
        // 同大小换过文件）→ 视为未安装，必须重新下载。
        let stale = Manifest {
            version: MANIFEST_VERSION,
            files: vec![VerifiedFile {
                name: file.target.to_string(),
                bytes: file.bytes,
                sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            }],
        };
        assert!(!is_installed(&layout, &stale, &file));

        let fresh = Manifest {
            version: MANIFEST_VERSION,
            files: vec![VerifiedFile {
                name: file.target.to_string(),
                bytes: file.bytes,
                sha256: file.sha256.to_string(),
            }],
        };
        assert!(is_installed(&layout, &fresh, &file));
        let _ = fs::remove_dir_all(&root);
    }

    /// 清单的 `version` 字段不参与兼容判定：旧版本号 + 结构有效 = 仍然是已安装。
    ///
    /// 这条守住的是「升级不能触发 241 MB 重下」——旧实现只要 version 不相等
    /// 就返回空清单，一次 MANIFEST_VERSION bump 就会让所有用户重下模型。
    #[test]
    fn stale_manifest_version_still_counts_as_installed() {
        let root = temp_root("manifest-version");
        let layout = Layout::new(&root);
        layout.ensure_dirs().unwrap();
        let file = assets::MODEL_FILES[1];
        let path = layout.file_path(file.target);
        fs::write(&path, vec![0u8; file.bytes as usize]).unwrap();

        // 手写一份 version=99 的清单：结构完全合法，只有版本号对不上。
        let stale = format!(
            r#"{{"version":99,"files":[{{"name":"{}","bytes":{},"sha256":"{}"}}]}}"#,
            file.target, file.bytes, file.sha256
        );
        fs::write(&layout.manifest, stale).unwrap();

        let manifest = read_manifest(&layout.manifest);
        assert_eq!(
            manifest.version, MANIFEST_VERSION,
            "读入后版本应被规整成当前常量"
        );
        assert!(
            is_installed(&layout, &manifest, &file),
            "版本号不同但结构有效时不得判为未安装"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn part_path_is_deterministic_and_under_downloads() {
        let layout = Layout::new(Path::new("C:\\data"));
        let part = layout.part_path(&assets::MODEL_FILES[0]);
        assert!(part.ends_with("downloads\\model.int8.onnx.part"));
        assert_eq!(part, layout.part_path(&assets::MODEL_FILES[0]));
    }

    #[test]
    fn hash_file_matches_known_digest() {
        let root = temp_root("hash");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("sample.bin");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"abc").unwrap();
        assert_eq!(
            hash_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn forget_manifest_is_idempotent() {
        let root = temp_root("forget");
        let layout = Layout::new(&root);
        layout.ensure_dirs().unwrap();
        forget_manifest(&layout.manifest).unwrap();
        write_manifest(&layout.manifest, &Manifest::default()).unwrap();
        forget_manifest(&layout.manifest).unwrap();
        assert!(!layout.manifest.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
