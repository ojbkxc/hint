//! 简历 / 项目资料的文件导入：读取本地文件并提取纯文本。
//!
//! 纯解析逻辑（编码识别、DOCX 文本抽取、清理）在 `hint_core::import`；
//! 这里只负责文件系统访问、格式分发与 PDF 文本提取。

use std::io::Read;
use std::path::Path;

use hint_core::import::{clean_extracted, decode_text, docx_text};

/// 单个资料文件上限：再大也不适合塞进提示词，且避免一次性读爆内存。
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// 返回给前端的文本上限（提示词本身只取前 25000 字符）。
const MAX_CHARS: usize = 200_000;
/// `word/document.xml` 解压后的上限：20 MB 的压缩包可以膨胀到数 GB，
/// 必须先限量再读，否则解压炸弹会把内存吃满。
const MAX_DOCX_XML_BYTES: u64 = 32 * 1024 * 1024;

pub fn document_text(path: &str) -> Result<String, String> {
    let path_buf = Path::new(path);
    let metadata = std::fs::metadata(path_buf).map_err(|error| format!("读取文件失败：{error}"))?;
    if !metadata.is_file() {
        return Err("请选择文件，而不是文件夹".to_string());
    }
    if metadata.len() == 0 {
        return Err("文件是空的".to_string());
    }
    if metadata.len() > MAX_BYTES {
        return Err(format!(
            "文件过大（{:.1} MB），上限 20 MB",
            metadata.len() as f64 / 1_048_576.0
        ));
    }
    let extension = path_buf
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = match extension.as_str() {
        "txt" | "md" | "markdown" | "text" => {
            let bytes =
                std::fs::read(path_buf).map_err(|error| format!("读取文件失败：{error}"))?;
            clean_extracted(&decode_text(&bytes))
        }
        "docx" => read_docx(path_buf)?,
        "pdf" => read_pdf(path_buf)?,
        "doc" => {
            return Err(
                "不支持旧版 .doc：请在 Word 中「另存为」.docx 或 .txt 后再导入".to_string(),
            );
        }
        "" => {
            return Err(
                "无法识别文件类型（没有扩展名）：请改用 .txt / .md / .docx / .pdf".to_string(),
            );
        }
        other => {
            return Err(format!(
                "不支持的文件类型 .{other}：支持 TXT / MD / DOCX / PDF"
            ));
        }
    };
    if text.trim().is_empty() {
        return Err("没有从文件里提取到文字（扫描版 PDF 需要先用 OCR 转成文本）".to_string());
    }
    Ok(truncate_chars(text, MAX_CHARS))
}

fn read_docx(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("打开 DOCX 失败：{error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("解析 DOCX 失败（文件可能已损坏）：{error}"))?;
    let entry = archive
        .by_name("word/document.xml")
        .map_err(|_| "DOCX 里缺少 word/document.xml（文件可能已损坏）".to_string())?;
    // 多读 1 字节只用于判断是否超限，正常文件读不满这个上限。
    let mut bytes = Vec::new();
    entry
        .take(MAX_DOCX_XML_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取 DOCX 内容失败：{error}"))?;
    if bytes.len() as u64 > MAX_DOCX_XML_BYTES {
        return Err(format!(
            "DOCX 内容过大（解压后超过 {} MB）：请在 Word 中另存为 TXT 后再导入",
            MAX_DOCX_XML_BYTES / 1_048_576
        ));
    }
    let xml = String::from_utf8(bytes)
        .map_err(|_| "DOCX 内容不是合法 UTF-8（文件可能已损坏）".to_string())?;
    Ok(docx_text(&xml))
}

fn read_pdf(path: &Path) -> Result<String, String> {
    let text = pdf_extract::extract_text(path).map_err(|error| {
        format!("解析 PDF 失败：{error}；扫描版 PDF 没有文本层，请改用 DOCX 或 TXT")
    })?;
    Ok(clean_extracted(&text))
}

fn truncate_chars(mut text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    let cut = text
        .char_indices()
        .nth(limit)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    text.truncate(cut);
    text.push_str("\n（文件内容过长，已截取前 20 万字符）");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 未超限时原样返回，且不追加任何提示。
    #[test]
    fn truncate_keeps_short_text_untouched() {
        let text = "短文本".to_string();
        let out = truncate_chars(text.clone(), MAX_CHARS);
        assert_eq!(out, text);
        assert!(!out.contains("已截取"), "未超限不该追加提示");
    }

    /// 超限时按**字符**截断（不按字节），并追加可读提示。
    ///
    /// 判据落在「字符边界安全」上：中文每字 3 字节，按字节切会切出非法 UTF-8。
    /// `char_indices().nth(limit)` 保证切点落在字符边界——这条测试钉住它。
    #[test]
    fn truncate_cuts_on_char_boundary_and_warns() {
        let text = "测".repeat(MAX_CHARS + 100);
        let out = truncate_chars(text, MAX_CHARS);

        assert!(out.contains("已截取前 20 万字符"), "超限要给出可读提示");
        // 截断后的正文应当恰好 MAX_CHARS 个字符
        let body = out.split('\n').next().unwrap_or("");
        assert_eq!(
            body.chars().count(),
            MAX_CHARS,
            "正文应恰好保留 {MAX_CHARS} 个字符"
        );
        assert!(!out.contains('\u{FFFD}'), "不该出现 UTF-8 替换字符");
    }

    /// 恰好等于上限时不截断（边界取 <=）。
    #[test]
    fn truncate_at_exact_limit_is_noop() {
        let text = "a".repeat(MAX_CHARS);
        let out = truncate_chars(text, MAX_CHARS);
        assert!(!out.contains("已截取"), "恰好等于上限不该截断");
        assert_eq!(out.chars().count(), MAX_CHARS);
    }

    /// 上限为 0 时不该 panic（防御性：调用方传 0 也不炸）。
    #[test]
    fn truncate_with_zero_limit_does_not_panic() {
        let out = truncate_chars("abc".to_string(), 0);
        assert!(out.contains("已截取") || out.is_empty());
    }

    /// 上限常量本身要有下界。
    ///
    /// 这条是「改一个字符会不会红」的守卫——把 MAX_CHARS 从 200_000 改成 200
    /// 会让所有长简历被截成两句话，而其余测试仍然全绿。
    ///
    /// 用 const block：断言的是编译期常量，clippy `assertions_on_constants` 要求这种写法。
    #[test]
    fn max_chars_is_large_enough_for_real_resumes() {
        const {
            assert!(
                MAX_CHARS >= 10_000,
                "MAX_CHARS 太小：真实简历 + 项目资料通常数千到上万字符"
            )
        };
    }

    /// 导入的**边界提示**要可行动。
    ///
    /// 真机验证导入边界时受环境限制（应用侧无法读取测试路径），无法触发这些分支。
    /// 用临时文件直接构造，把每条提示的判据锁住——提示文案改错、分支判断改错，
    /// 都会让测试变红。
    ///
    /// ## 判据
    ///
    /// 每条错误都要说清**发生了什么 + 该怎么做**，而不是只报"失败"。
    #[test]
    fn document_text_boundary_messages() {
        let dir = std::env::temp_dir().join(format!("hint-import-boundary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");

        let mk = |name: &str, content: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, content).expect("写测试文件");
            p.to_string_lossy().to_string()
        };

        // 空文件 → 明确说"是空的"
        let empty = mk("empty.txt", b"");
        let err = document_text(&empty).unwrap_err();
        assert!(err.contains("空"), "空文件应提示「空」，实际：{err}");

        // 没有扩展名 → 给出可用的格式
        let noext = mk("noextension", b"some text");
        let err = document_text(&noext).unwrap_err();
        assert!(err.contains("扩展名"), "无扩展名应提示扩展名，实际：{err}");
        assert!(err.contains(".txt"), "应给出可用格式，实际：{err}");

        // 未知扩展名 → 点明不支持，列出支持的
        let unknown = mk("file.xyz", b"some text");
        let err = document_text(&unknown).unwrap_err();
        assert!(err.contains(".xyz"), "应点明扩展名，实际：{err}");
        assert!(err.contains("TXT"), "应列出支持的格式，实际：{err}");

        // 旧版 .doc → 给出替代做法
        let olddoc = mk("legacy.doc", b"some text");
        let err = document_text(&olddoc).unwrap_err();
        assert!(err.contains(".doc"), "应点明扩展名，实际：{err}");
        assert!(err.contains("另存为"), "应给出替代做法，实际：{err}");

        // 文件夹（不是文件）→ 明确说选错了
        let folder = dir.join("a_folder");
        std::fs::create_dir(&folder).expect("建文件夹");
        let err = document_text(&folder.to_string_lossy()).unwrap_err();
        assert!(err.contains("文件夹"), "应提示选了文件夹，实际：{err}");

        // 内容为空（有扩展名但提取不到文字）→ 给出扫描版 PDF 的处置
        let blank_pdf = mk("blank.pdf", b"not a real pdf");
        let err = document_text(&blank_pdf).unwrap_err();
        assert!(
            err.contains("没有从文件里提取到文字"),
            "应说明没提取到文字，实际：{err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 超大文件的提示要带**实际大小**，不是固定文案。
    ///
    /// 21 MB 的文件提示「文件过大（20.0 MB），上限 20 MB」——用户能看到自己
    /// 那个文件到底多大，才知道该删多少。固定文案会让不同大小的文件得到一样的
    /// 提示，用户无法判断要清理多少。
    #[test]
    fn oversized_file_message_includes_actual_size() {
        let dir = std::env::temp_dir().join(format!("hint-import-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");

        // 造一个刚好超过上限的文件（20MB + 1KB）
        let size = MAX_BYTES + 1024;
        let path = dir.join("big.txt");
        {
            let mut f = std::fs::File::create(&path).expect("建大文件");
            use std::io::Write;
            let chunk = vec![b'x'; 1024 * 1024];
            for _ in 0..(size / (1024 * 1024)) {
                f.write_all(&chunk).expect("写");
            }
            let remaining = size % (1024 * 1024);
            if remaining > 0 {
                f.write_all(&chunk[..remaining]).expect("写余量");
            }
        }

        let err = document_text(&path.to_string_lossy()).unwrap_err();
        assert!(err.contains("过大"), "应提示过大，实际：{err}");
        assert!(err.contains("20 MB"), "应说明上限，实际：{err}");
        // 实际大小应出现在提示里（约 20.0 MB）
        assert!(err.contains("20.0 MB"), "应带实际大小，实际：{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
