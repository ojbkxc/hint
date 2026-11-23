//! 简历 / 项目资料的文件导入：读取本地文件并提取纯文本。
//!
//! 纯解析逻辑（编码识别、DOCX 文本抽取、清理）在 `hint_core::import`；
//! 这里只负责文件系统访问、格式分发与 PDF 文本提取。

use std::io::Read;
use std::path::Path;

use hint_core::import::{clean_extracted, decode_text, docx_text};

/// 单个资料文件上限：再大也不适合塞进提示词，且避免一次性读爆内存。
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// 返回给前端的文本上限（提示词本身只取前 6000 字符）。
const MAX_CHARS: usize = 200_000;
/// `word/document.xml` 解压后的上限：20 MB 的压缩包可以膨胀到数 GB，
/// 必须先限量再读，否则解压炸弹会把内存吃满。
const MAX_DOCX_XML_BYTES: u64 = 32 * 1024 * 1024;

pub fn document_text(path: &str) -> Result<String, String> {
    let path_buf = Path::new(path);
    let metadata =
        std::fs::metadata(path_buf).map_err(|error| format!("读取文件失败：{error}"))?;
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
