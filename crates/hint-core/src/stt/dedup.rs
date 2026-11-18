//! 补录文本去重：判断补录片段是否已被实时转录覆盖。
//!
//! 用字符 2-gram 覆盖率（与 `pipeline::is_echo` 同思路）。纯函数，便于单测。

use std::collections::HashSet;

/// 归一化：只保留字母数字（含中文），转小写。
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 生成字符 2-gram 集合。
fn bigrams(text: &str) -> HashSet<(char, char)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return HashSet::new();
    }
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

/// 补录文本是否已被实时转录覆盖（2-gram 覆盖率 ≥ `threshold`）。
///
/// - 补录文本归一化后少于 3 个字符 → false（太短，无可比性）；
/// - `live_texts` 为空或 2-gram 并集为空 → false；
/// - 覆盖率 = 补录 2-gram 中出现在实时文本并集里的比例。
pub fn is_covered_by_live(text: &str, live_texts: &[String], threshold: f64) -> bool {
    let normalized = normalize(text);
    if normalized.chars().count() < 3 {
        return false;
    }
    if live_texts.is_empty() {
        return false;
    }
    let candidate = bigrams(&normalized);
    if candidate.is_empty() {
        return false;
    }
    let mut live_union = HashSet::new();
    for live in live_texts {
        live_union.extend(bigrams(&normalize(live)));
    }
    if live_union.is_empty() {
        return false;
    }
    let covered = candidate
        .iter()
        .filter(|bigram| live_union.contains(bigram))
        .count();
    (covered as f64) / (candidate.len() as f64) >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_covered() {
        assert!(is_covered_by_live(
            "我们明天上午开会",
            &["我们明天上午开会".into()],
            0.7
        ));
    }

    #[test]
    fn mostly_overlapping_is_covered() {
        // 补录多一个「吧」，2-gram 覆盖率约 7/8 = 0.875。
        assert!(is_covered_by_live(
            "我们明天上午开会吧",
            &["我们明天上午开会".into()],
            0.7
        ));
    }

    #[test]
    fn different_text_is_not_covered() {
        assert!(!is_covered_by_live(
            "完全不同的一段新内容",
            &["我们明天上午开会".into()],
            0.7
        ));
    }

    #[test]
    fn short_text_is_not_covered() {
        assert!(!is_covered_by_live("好", &["好的".into()], 0.7));
    }

    #[test]
    fn empty_live_list_is_not_covered() {
        assert!(!is_covered_by_live("我们明天上午开会", &[], 0.7));
    }
}
