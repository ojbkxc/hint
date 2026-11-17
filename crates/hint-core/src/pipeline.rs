//! 问题判定、回声过滤、提示词组装。
//!
//! 本模块全部是纯函数：不访问网络、不操作音频设备，可以在 STT 收段后的
//! 热路径上同步调用。[`looks_like_question`] 只做字符串匹配（微秒级），
//! [`is_echo`] 做字符 2-gram 集合比较（亚毫秒级）。
//!
//! 已知取舍：问题判定是启发式的，追求「宁可多判、不要漏判」。例如面试官
//! 陈述句里出现「为什么」「可以」等词时可能被误判为问题；麦克风通道本就
//! 不触发生成（见 [`should_generate`]），影响面主要在系统声通道的偶发误触。

use std::borrow::Cow;
use std::collections::HashSet;

pub use crate::llm::{Turn, TurnRole};
use crate::settings::{Settings, TriggerMode};

/// 回声判定阈值：麦克风文本的 2-gram 有 70% 以上出现在面试官近期文本中，
/// 判定为扬声器回声。
pub const ECHO_OVERLAP_THRESHOLD: f64 = 0.70;

/// 麦克风文本少于 6 个汉字时不判回声（太短，重合度没有统计意义）。
pub const MIN_ECHO_HAN_CHARS: usize = 6;

/// 每次组装 user prompt 时携带的历史轮数上限。
pub const PROMPT_HISTORY_TURNS: usize = 6;

/// 对话角色的中文标签（提示词与转录面板用）。
pub fn role_label(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Interviewer => "面试官",
        TurnRole::Me => "我",
    }
}

/// 中文问句标志词：命中任意一个即视为问题候选。
const CN_QUESTION_MARKERS: &[&str] = &[
    "吗",
    "呢",
    "什么",
    "怎么",
    "为什么",
    "如何",
    "能不能",
    "可以",
    "请介绍",
    "介绍",
    "说说",
    "讲讲",
    "描述",
    "举例",
    "有没有",
    "是不是",
    "对不对",
    "多少",
    "哪",
    "多久",
    "谈谈",
    "聊聊",
    "谈一谈",
    "聊一聊",
    "说一说",
    "讲一讲",
    "说一下",
    "讲一下",
    "请问",
    "想问",
    "是否",
    "会不会",
    "要不要",
    "行不行",
    "好不好",
    "你觉得",
    "你认为",
];

/// 这些片段出现时先删掉再找标志词，避免「没什么」「我先介绍一下」这类
/// 陈述句被误判成问题。
const CN_NON_QUESTION_FRAGMENTS: &[&str] = &[
    "没有什么",
    "没什么",
    "不怎么",
    "我先介绍一下",
    "我来介绍一下",
    "我介绍一下",
    "我先说下",
    "我先讲下",
];

/// 寒暄、应答与纯语气词。整句只由这些词组成时直接排除。
/// 注意按「最长前缀优先」逐段剥离，因此列表顺序不影响结果。
const GREETINGS: &[&str] = &[
    "thank you very much",
    "good afternoon",
    "good morning",
    "good evening",
    "谢谢大家",
    "谢谢你",
    "谢谢您",
    "辛苦大家",
    "上午好",
    "下午好",
    "中午好",
    "早上好",
    "晚上好",
    "大家好",
    "你们好",
    "thank you",
    "thanks",
    "hello",
    "好的好的",
    "没问题",
    "明白了",
    "收到",
    "可以",
    "是的",
    "对的",
    "好的",
    "嗯嗯",
    "嗯呐",
    "您好",
    "你好",
    "辛苦",
    "谢谢",
    "多谢",
    "感谢",
    "再见",
    "拜拜",
    "晚安",
    "okay",
    "ok",
    "hi",
    "hey",
    "bye",
    "了解",
    "明白",
    "行",
    "好",
    "对",
    "是",
    "嗯",
    "呃",
    "额",
    "啊",
    "哦",
    "噢",
    "喔",
    "唉",
    "诶",
    "喂",
];

/// 英文 wh- 开头的疑问词（只认句首）。
const EN_WH_WORDS: &[&str] = &[
    "what", "why", "how", "when", "where", "who", "whom", "whose", "which",
];

/// 常见的英文疑问句开头（前两个词）。
const EN_QUESTION_OPENERS: &[&str] = &[
    "can you",
    "could you",
    "would you",
    "will you",
    "do you",
    "did you",
    "are you",
    "were you",
    "have you",
    "tell me",
    "walk me",
    "give me",
    "show me",
];

/// 句首祈使动词，等价于「请介绍/请说明」。
const EN_IMPERATIVE_OPENERS: &[&str] = &["describe", "explain", "tell", "walk", "share"];

/// 判断一段文本是否像面试官提出的问题。
///
/// 纯字符串匹配，不调用 LLM、不引入正则编译开销，单次耗时在微秒级。
/// 判定顺序：
/// 1. 空文本或纯标点 → 不是问题；
/// 2. 整句只是寒暄/应答/语气词 → 不是问题；
/// 3. 出现问号（半角或全角）→ 是问题；
/// 4. 删除「没什么」等否定片段后命中中文标志词 → 是问题；
/// 5. 英文以 wh- 词、疑问句式或祈使动词开头 → 是问题。
pub fn looks_like_question(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    if is_pure_greeting(text) {
        return false;
    }
    if text.contains('?') || text.contains('？') {
        return true;
    }

    let scan = remove_non_question_fragments(text);
    if CN_QUESTION_MARKERS
        .iter()
        .any(|marker| scan.contains(marker))
    {
        return true;
    }

    looks_like_english_question(text)
}

/// 删除陈述句里的否定/自述片段，返回可能借用的字符串。
fn remove_non_question_fragments(text: &str) -> Cow<'_, str> {
    let mut result: Option<String> = None;
    for fragment in CN_NON_QUESTION_FRAGMENTS {
        let current = result.as_deref().unwrap_or(text);
        if current.contains(fragment) {
            result = Some(current.replace(fragment, ""));
        }
    }
    match result {
        Some(owned) => Cow::Owned(owned),
        None => Cow::Borrowed(text),
    }
}

/// 英文问句判定：问号已在调用方处理；这里只看句首结构。
fn looks_like_english_question(text: &str) -> bool {
    if !text.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let lowered = text.to_lowercase();
    let words: Vec<&str> = lowered
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '\''))
        .filter(|word| !word.is_empty())
        .collect();
    let Some(first) = words.first().copied() else {
        return false;
    };
    if EN_WH_WORDS.contains(&first) {
        return true;
    }
    if EN_IMPERATIVE_OPENERS.contains(&first) {
        return true;
    }
    if words.len() >= 2 {
        let pair = format!("{} {}", words[0], words[1]);
        if EN_QUESTION_OPENERS.contains(&pair.as_str()) {
            return true;
        }
    }
    false
}

/// 整句是否只由寒暄、应答或语气词组成。
///
/// 实现方式：反复剥离「最长匹配前缀」，直到无法继续；若字符串恰好被剥空，
/// 则说明它是纯寒暄。这样「好的谢谢」「嗯嗯，好的」这类组合也能识别。
fn is_pure_greeting(text: &str) -> bool {
    let mut rest = compact(text);
    if rest.is_empty() {
        // 纯空白或纯标点，没有语义，按无意义输入处理。
        return true;
    }
    loop {
        let mut best: Option<&str> = None;
        for token in GREETINGS {
            if rest.starts_with(token) {
                let longer = match best {
                    Some(current) => token.chars().count() > current.chars().count(),
                    None => true,
                };
                if longer {
                    best = Some(token);
                }
            }
        }
        let Some(token) = best else {
            return false;
        };
        rest = rest[token.len()..].to_string();
        if rest.is_empty() {
            return true;
        }
    }
}

/// 去除空白与标点，并把 ASCII 转小写。用于寒暄识别与回声比较。
fn compact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_whitespace() || is_punctuation(ch) {
            continue;
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// 判断字符是否为标点（ASCII 标点 + 常见中日韩/全角标点）。
fn is_punctuation(ch: char) -> bool {
    if ch.is_ascii_punctuation() {
        return true;
    }
    matches!(
        ch as u32,
        0x2000..=0x206F // 通用标点：– — ‘ ’ “ ” …
            | 0x3000..=0x303F // 中日韩标点：、。〈〉《》「」【】
            | 0xFE10..=0xFE6F // 竖排/小写变体标点
            | 0xFF01..=0xFF0F // 全角 ！＂＃…／
            | 0xFF1A..=0xFF20 // 全角 ：；＜＝＞？＠
            | 0xFF3B..=0xFF40 // 全角 ［＼］＾＿｀
            | 0xFF5B..=0xFF65 // 全角 ｛｜｝～｟｠｡｢｣､･
    ) || ch == '·'
}

/// 判断字符是否属于 CJK 统一表意文字（用于「不少于 6 个汉字」的门槛）。
fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
    )
}

/// 麦克风段是否为扬声器回声。
///
/// 规则（对应产品文档第 6 节）：
/// - 去空白与标点后，麦克风文本少于 [`MIN_ECHO_HAN_CHARS`] 个汉字 → 不判回声；
/// - 中文按字符 2-gram 去重集合计算覆盖率；
/// - 麦克风 2-gram 有 ≥ [`ECHO_OVERLAP_THRESHOLD`] 出现在最近面试官文本中 → 回声。
///
/// `recent_interviewer` 传入面试官最近若干段（通常是最近 8 秒）的识别文本。
pub fn is_echo(mic_text: &str, recent_interviewer: &[&str]) -> bool {
    let mic = compact(mic_text);
    if mic.chars().filter(|ch| is_cjk(*ch)).count() < MIN_ECHO_HAN_CHARS {
        return false;
    }
    let mic_grams = bigrams(&mic);
    if mic_grams.is_empty() {
        return false;
    }

    let mut interviewer_grams: HashSet<(char, char)> = HashSet::new();
    for text in recent_interviewer {
        interviewer_grams.extend(bigrams(&compact(text)));
    }
    if interviewer_grams.is_empty() {
        return false;
    }

    let matched = mic_grams
        .iter()
        .filter(|gram| interviewer_grams.contains(*gram))
        .count();
    matched as f64 / mic_grams.len() as f64 >= ECHO_OVERLAP_THRESHOLD
}

/// 字符 2-gram 去重集合。
fn bigrams(text: &str) -> HashSet<(char, char)> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .windows(2)
        .map(|window| (window[0], window[1]))
        .collect()
}

/// 组装 system prompt。
///
/// 复用 [`Settings::system_prompt`]（角色、回答风格、简历、项目资料），
/// 再补充实时口述场景的约束：答案要能直接照念、资料未覆盖时不许编造。
pub fn build_system_prompt(settings: &Settings) -> String {
    let mut prompt = settings.system_prompt();
    prompt.push_str(
        "\n补充要求：\n\
         - 候选人正在实时面试，回答会被直接照念，请用短句，不要输出 Markdown 标题。\n\
         - 只依据上面的简历与项目资料作答；资料没有覆盖的内容，用通用经验诚实回答，不要编造。\n",
    );
    prompt
}

/// 组装 user prompt：最近 [`PROMPT_HISTORY_TURNS`] 条对话 + 当前问题。
///
/// 输出形如：
/// ```text
/// 当前对话（最近 6 轮）：
/// 面试官：聊聊你的项目
/// 我：我主要负责自动化测试……
///
/// 面试官刚刚问：这个项目的难点是什么
/// 请给出回答。
/// ```
pub fn build_user_prompt(question: &str, history: &[Turn]) -> String {
    let start = history.len().saturating_sub(PROMPT_HISTORY_TURNS);
    let recent = &history[start..];

    let mut prompt = String::new();
    if !recent.is_empty() {
        prompt.push_str(&format!("当前对话（最近 {} 轮）：\n", recent.len()));
        for turn in recent {
            prompt.push_str(role_label(turn.role));
            prompt.push('：');
            prompt.push_str(turn.text.trim());
            prompt.push('\n');
        }
        prompt.push('\n');
    }
    prompt.push_str("面试官刚刚问：");
    prompt.push_str(question.trim());
    prompt.push_str("\n请给出回答。");
    prompt
}

/// 是否应根据触发方式自动生成答案。
///
/// - `TriggerMode::Auto`：只有判定为问题才生成；
/// - `TriggerMode::Manual`：永远不自动生成，由热键显式触发。
pub fn should_generate(trigger: TriggerMode, looks_like_question: bool) -> bool {
    match trigger {
        TriggerMode::Auto => looks_like_question,
        TriggerMode::Manual => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AnswerStyle;

    fn assert_question(text: &str) {
        assert!(looks_like_question(text), "应判为问题：{text}");
    }

    fn assert_not_question(text: &str) {
        assert!(!looks_like_question(text), "不应判为问题：{text}");
    }

    /// 测试用：构造面试官回合（复用 `llm::Turn`）。
    fn interviewer(text: impl Into<String>) -> Turn {
        Turn {
            role: TurnRole::Interviewer,
            text: text.into(),
        }
    }

    /// 测试用：构造候选人回合。
    fn me(text: impl Into<String>) -> Turn {
        Turn {
            role: TurnRole::Me,
            text: text.into(),
        }
    }

    /// 中文问句用例（要求至少 25 条中/英问题与非问题；这里中文部分已 30+）。
    #[test]
    fn detects_chinese_questions() {
        let cases = [
            "你能说说这个项目的难点吗？",
            "为什么选择用 Rust 来做这个项目？",
            "你平时怎么保证测试用例的覆盖率？",
            "请介绍一下你在上家公司的工作经历。",
            "能不能具体讲讲你做过哪些自动化测试？",
            "如何定位线上出现的性能问题？",
            "有没有接触过容器化和 CI/CD？",
            "这个方案是不是你们团队一起定的？",
            "你在项目里主要负责哪一部分？",
            "你觉得这个设计对不对？",
            "团队一共有多少人？",
            "说说你遇到过最棘手的一次线上故障。",
            "举例说一下你是怎么推进跨部门协作的。",
            "这个接口一次返回多少条数据？",
            "能描述一下你们的质量保障流程吗",
            "你有什么想问我的吗？",
            "你对我们公司了解多少？",
            "你在上一家公司主要负责什么工作？",
            "具体是怎么实现的？",
            "你觉得还可以再优化一下，你觉得呢？",
            "这个流程大概需要多久？",
            "请简单做个自我介绍。",
            "谈谈你对测试左移的理解。",
            "讲一下你们发布流程里怎么控制风险。",
            "请问你们现在用的是什么消息队列？",
            "是否了解过契约测试？",
            "你之前做过类似的项目吗",
            "我先问一下，你们的灰度策略是怎样的？",
            "你们线上问题的响应机制行不行得通？",
            "如果要你来改进，你会先动哪里？",
        ];
        for text in cases {
            assert_question(text);
        }
    }

    /// 中文非问题用例：陈述句、寒暄、应答、语气词。
    #[test]
    fn rejects_chinese_non_questions() {
        let cases = [
            "我主要负责自动化测试和接口测试。",
            "这个项目从去年三月开始，做了八个月。",
            "我们团队一共五个人。",
            "好的，我明白了。",
            "嗯嗯。",
            "谢谢。",
            "你好。",
            "没问题，我继续讲。",
            "没什么特别的地方。",
            "那就不太清楚了。",
            "我先介绍一下我的工作经历。",
            "今天天气不错。",
            "我们用的是 Python 和 Playwright。",
            "当时压力挺大的。",
            "我先说下背景吧。",
            "这个接口返回一千条数据。",
            "我们每周做一次回归。",
            "我平时喜欢看书和跑步。",
            "这件事我负责跟进。",
            "好的谢谢，我这边没有补充了。",
        ];
        for text in cases {
            assert_not_question(text);
        }
    }

    #[test]
    fn detects_english_questions() {
        for text in [
            "What is your biggest challenge?",
            "Can you walk me through the architecture",
            "How do you handle conflicts in a team",
            "Tell me about a difficult bug you fixed.",
            "Why did you choose Rust for this service?",
            "Describe your testing process.",
        ] {
            assert_question(text);
        }
    }

    #[test]
    fn rejects_english_non_questions() {
        for text in [
            "This project uses SQLite for storage.",
            "I know how the pipeline works.",
            "The team has five engineers.",
            "We shipped the feature last month.",
        ] {
            assert_not_question(text);
        }
    }

    #[test]
    fn rejects_empty_and_greeting_only_input() {
        assert_not_question("");
        assert_not_question("   ");
        assert_not_question("……");
        assert_not_question("嗯、啊、哦。");
        assert_not_question("OK");
        assert_not_question("Thanks!");
        assert_question("你好，可以先做个自我介绍吗");
    }

    #[test]
    fn echo_exact_sentence_is_detected() {
        let text = "你能详细说说这个项目的难点吗";
        assert!(is_echo(text, &[text]));
    }

    #[test]
    fn echo_ignores_punctuation_and_spaces() {
        assert!(is_echo(
            "你好，今天面试的流程是什么？",
            &["你好 今天面试的流程是什么"]
        ));
    }

    #[test]
    fn short_mic_text_is_not_echo() {
        // 5 个汉字，低于 6 字门槛。
        assert!(!is_echo("我们今天面", &["我们今天面试"]));
        assert!(!is_echo("好的", &["好的"]));
    }

    #[test]
    fn overlap_below_threshold_is_not_echo() {
        // mic 2-gram：我们/们明/明天/天上/上午/午开/开会，命中 3/7 ≈ 0.43。
        assert!(!is_echo("我们明天上午开会", &["我们今天上午面试"]));
    }

    #[test]
    fn overlap_just_above_threshold_is_echo() {
        // mic 2-gram 7 个，命中 5 个 ≈ 0.714，刚过 0.70。
        assert!(is_echo("我们今天面试很好", &["我们今天面试的流程是这样的"]));
    }

    #[test]
    fn six_chars_with_partial_match_is_not_echo() {
        // mic 2-gram：我们/们今/今天/天面/面试，命中 3/5 = 0.6。
        assert!(!is_echo("我们今天面试", &["我们今天开会讨论方案"]));
    }

    #[test]
    fn paraphrase_is_not_echo() {
        assert!(!is_echo(
            "这个项目的主要难点在哪里",
            &["这个项目难点是什么"]
        ));
    }

    #[test]
    fn empty_interviewer_history_is_not_echo() {
        assert!(!is_echo("你能详细说说这个项目的难点吗", &[]));
        assert!(!is_echo("你能详细说说这个项目的难点吗", &["", "   "]));
    }

    #[test]
    fn echo_matches_any_recent_interviewer_segment() {
        assert!(is_echo(
            "我们来聊聊你们的项目难点和解决思路",
            &["今天天气不错", "我们来聊聊你们的项目难点和解决思路"]
        ));
    }

    #[test]
    fn english_mic_text_is_not_echo() {
        assert!(!is_echo(
            "hello can you hear me",
            &["hello can you hear me"]
        ));
    }

    #[test]
    fn empty_mic_text_is_not_echo() {
        assert!(!is_echo("", &["你能详细说说这个项目的难点吗"]));
        assert!(!is_echo("？？？", &["你能详细说说这个项目的难点吗"]));
    }

    fn sample_settings() -> Settings {
        Settings {
            resume: "候选人：张三，五年测试开发经验。".to_string(),
            projects: "项目 A：基于 Playwright 的 UI 自动化平台。".to_string(),
            ..Settings::default()
        }
    }

    #[test]
    fn system_prompt_contains_role_and_style() {
        let prompt = build_system_prompt(&Settings::default());
        assert!(prompt.contains("面试实时提词器"));
        assert!(prompt.contains("先给结论"));
        assert!(prompt.contains("补充要求"));
    }

    #[test]
    fn system_prompt_injects_resume_and_projects() {
        let prompt = build_system_prompt(&sample_settings());
        assert!(prompt.contains("候选人简历"));
        assert!(prompt.contains("五年测试开发经验"));
        assert!(prompt.contains("项目资料"));
        assert!(prompt.contains("Playwright"));
    }

    #[test]
    fn system_prompt_respects_outline_style() {
        let settings = Settings {
            answer_style: AnswerStyle::Outline,
            ..Settings::default()
        };
        let prompt = build_system_prompt(&settings);
        assert!(prompt.contains("5-7 条要点"));
    }

    #[test]
    fn user_prompt_formats_history_and_question() {
        let history = vec![interviewer("聊聊你的项目"), me("我主要负责自动化测试。")];
        let prompt = build_user_prompt("这个项目的难点是什么", &history);
        assert!(prompt.contains("当前对话（最近 2 轮）"));
        assert!(prompt.contains("面试官：聊聊你的项目"));
        assert!(prompt.contains("我：我主要负责自动化测试。"));
        assert!(prompt.contains("面试官刚刚问：这个项目的难点是什么"));
        assert!(prompt.contains("请给出回答。"));
    }

    #[test]
    fn user_prompt_keeps_only_last_six_turns() {
        let history: Vec<Turn> = (1..=8)
            .map(|index| interviewer(format!("t{index}")))
            .collect();
        let prompt = build_user_prompt("最后一个问题", &history);
        assert!(prompt.contains("最近 6 轮"));
        assert!(prompt.contains("t3"));
        assert!(prompt.contains("t8"));
        assert!(!prompt.contains("t1"));
        assert!(!prompt.contains("t2"));
    }

    #[test]
    fn user_prompt_works_without_history() {
        let prompt = build_user_prompt("介绍一下自己", &[]);
        assert!(!prompt.contains("当前对话"));
        assert!(prompt.contains("面试官刚刚问：介绍一下自己"));
    }

    #[test]
    fn role_labels_are_chinese() {
        assert_eq!(role_label(TurnRole::Interviewer), "面试官");
        assert_eq!(role_label(TurnRole::Me), "我");
    }

    #[test]
    fn should_generate_follows_trigger_mode() {
        assert!(should_generate(TriggerMode::Auto, true));
        assert!(!should_generate(TriggerMode::Auto, false));
        assert!(!should_generate(TriggerMode::Manual, true));
        assert!(!should_generate(TriggerMode::Manual, false));
    }
}
