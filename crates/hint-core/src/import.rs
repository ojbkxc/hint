//! 简历 / 项目资料的纯文本提取（与平台无关的部分）。
//!
//! 文件读取和 PDF 解析在壳层（`src-tauri/src/import.rs`），这里只放
//! 可以单测的纯函数：编码识别、DOCX 的 `word/document.xml` 文本抽取、
//! 文本清理。壳层负责把字节读进来再调用这里。

/// 把用户文件的内容解码成字符串。
///
/// 先按 UTF-8 解码（含 BOM 自动剥离），失败时回退 GBK——中文 Windows
/// 上的老 txt 很多是 GBK 编码。两者都失败时用有损 UTF-8 保证不丢内容。
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let (decoded, _, had_errors) = encoding_rs::GBK.decode(bytes);
    if !had_errors {
        return decoded.into_owned();
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// 从 DOCX 的 `word/document.xml` 里抽取纯文本。
///
/// 不引入完整 XML 解析器：Word 的正文文本都在 `<w:t>` 里，段落边界是
/// `</w:p>`。按段落切分后取所有 `<w:t>` 内容并解码 XML 实体即可。
pub fn docx_text(xml: &str) -> String {
    // 自闭合的空段落 `<w:p/>`（Word 常用它制造空行）先展开成
    // `<w:p></w:p>`，这样按 `</w:p>` 分割时才会保留这个空行。
    let expanded = expand_self_closing_paragraphs(xml);
    let mut paragraphs: Vec<String> = Vec::new();
    for chunk in expanded.split("</w:p>") {
        let mut paragraph = String::new();
        let mut rest = chunk;
        while let Some(start) = find_tag(rest, "<w:t") {
            let after = &rest[start..];
            let Some(gt) = after.find('>') else { break };
            // 自闭合 `<w:t/>` 没有文本内容。
            if after[..=gt].ends_with("/>") {
                rest = &after[gt + 1..];
                continue;
            }
            let tail = &after[gt + 1..];
            let Some(end) = tail.find("</w:t>") else {
                // 未闭合的 <w:t>：把剩余内容收下，避免吞文字。
                paragraph.push_str(&decode_xml_entities(tail));
                break;
            };
            paragraph.push_str(&decode_xml_entities(&tail[..end]));
            rest = &tail[end + 6..];
        }
        paragraphs.push(paragraph);
    }
    clean_extracted(&paragraphs.join("\n"))
}

/// 把形如 `<w:p/>`、`<w:p w:rsidR="…"/>` 的自闭合段落展开成
/// `<w:p></w:p>`，保证空段落不会被丢弃。
fn expand_self_closing_paragraphs(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = find_tag(rest, "<w:p") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(gt) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let tag = &after[..=gt];
        if tag.ends_with("/>") {
            out.push_str("<w:p></w:p>");
        } else {
            out.push_str(tag);
        }
        rest = &after[gt + 1..];
    }
    out.push_str(rest);
    out
}

/// 查找完整标签名：`<w:t` 后面必须紧跟 `>`、空格或 `/`，
/// 避免把 `<w:tab/>`、`<w:tc>` 之类的相邻标签误当成文本标签。
fn find_tag(text: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(position) = text[from..].find(tag) {
        let start = from + position;
        let rest = &text[start + tag.len()..];
        if matches!(rest.chars().next(), Some('>') | Some(' ') | Some('/')) {
            return Some(start);
        }
        from = start + tag.len();
    }
    None
}

/// 清理抽取出来的文本：去行尾空白、压缩连续空行、统一换行。
pub fn clean_extracted(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = Vec::new();
    let mut blank_run = 0usize;
    for line in normalized.split('\n') {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blank_run += 1;
            // 最多保留一个空行，避免 PDF 抽出大量空白页间隔。
            if blank_run > 1 {
                continue;
            }
            lines.push("");
        } else {
            blank_run = 0;
            lines.push(trimmed);
        }
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn decode_xml_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                // 未知实体原样保留，避免吞内容。
                out.push_str(&tail[..=semi]);
            }
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_text_prefers_utf8_then_gbk() {
        assert_eq!(decode_text("简历：Java 开发".as_bytes()), "简历：Java 开发");
        // UTF-8 BOM 被剥离。
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice("简历".as_bytes());
        assert_eq!(decode_text(&with_bom), "简历");
        // GBK 编码的「简历」：0xBC 0xF2 0xC0 0xFA。
        assert_eq!(decode_text(&[0xBC, 0xF2, 0xC0, 0xFA]), "简历");
    }

    #[test]
    fn docx_text_extracts_paragraphs_and_entities() {
        let xml = r#"<w:document><w:body>
            <w:p><w:r><w:t>第一段</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">第二段 </w:t></w:r><w:r><w:t>&amp;实体</w:t></w:r></w:p>
            <w:p/>
            <w:p><w:r><w:t>第三段</w:t></w:r></w:p>
        </w:body></w:document>"#;
        assert_eq!(docx_text(xml), "第一段\n第二段 &实体\n\n第三段");
    }

    #[test]
    fn docx_text_handles_numeric_entities_and_missing_close_tag() {
        let xml = "<w:p><w:t>A&#32;B&#x4E2D;</w:t></w:p>";
        assert_eq!(docx_text(xml), "A B中");
        // 未闭合的 <w:t> 不 panic，也不吞后续内容。
        let broken = "<w:p><w:t>未闭合</w:p><w:p><w:t>下一段</w:t></w:p>";
        assert_eq!(docx_text(broken), "未闭合\n下一段");
    }

    #[test]
    fn docx_text_ignores_similar_tag_names() {
        // <w:tc>（单元格）与 <w:tab/>（制表符）不是文本标签，不能被当成 <w:t。
        let xml = "<w:p><w:tc><w:t>单元格</w:t></w:tc><w:tab/><w:t>正文</w:t></w:p>";
        assert_eq!(docx_text(xml), "单元格正文");
    }

    #[test]
    fn clean_extracted_collapses_blank_runs_and_trims_trailing() {
        assert_eq!(
            clean_extracted("a\r\n\r\n\r\nb  \n\n"),
            "a\n\nb"
        );
        assert_eq!(clean_extracted("\n\n"), "");
    }
}
