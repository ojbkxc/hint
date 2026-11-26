//! 面试档案：把多份文档整理成「可检索的紧凑事实片段」。
//!
//! ## 为什么需要这个
//!
//! 1.1.x 的做法是把简历与项目资料（各截断 6000 字）**每次请求全量塞进 system
//! prompt**。中文 12000 字约 8–12k tokens，而首字延迟（TTFT）与 prompt 长度近似
//! 成正比——这是「点了提问要等很久」的主要来源。
//!
//! 但面试里每个问题其实只用得上一小部分资料：问「做过什么自动化」只要项目里的
//! 测试部分，问「为什么离职」只要软性信息。全量发送纯属浪费。
//!
//! ## 结构
//!
//! 整理阶段（离线、用户点「整理」）把多份文档压成若干 [`Fact`]，每条是一行紧凑
//! 事实 + 标签 + 出处。运行时按问题检索 top-N 拼接，总预算受控。
//!
//! ## 格式选择
//!
//! 用 `|` 分隔的紧凑行，**不用 Markdown 表格**：表格的管道符与对齐空格白烧 token，
//! 同等信息量下实测差 30–50%。每行一条事实也便于按预算裁剪——表格缺一行就散了。

use serde::{Deserialize, Serialize};

/// 一条事实的类型。检索时按类型给不同权重（`Basics`/`Skill` 常驻，
/// `Story`/`Qa` 只在命中时出现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactKind {
    /// 基本信息：姓名、年限、岗位、学历。**常驻**。
    Basics,
    /// 技能：`技能名:熟练度|证据:项目编号`。**常驻**（短）。
    Skill,
    /// 项目：一段经历的做法/成果/难点。
    Project,
    /// 故事：STAR 形态的可用素材（最难的问题、最大成就、冲突处理…）。
    Story,
    /// 常见问答：离职原因、期望薪资、反问等。
    Qa,
}

impl FactKind {
    /// 常驻类型：无论问什么都带上（信息密度高、字数少）。
    pub fn is_always_included(self) -> bool {
        matches!(self, FactKind::Basics | FactKind::Skill)
    }

    /// 检索权重。项目与故事最能支撑回答，问答次之。
    fn weight(self) -> f32 {
        match self {
            FactKind::Basics => 1.0,
            FactKind::Skill => 1.0,
            FactKind::Project => 1.5,
            FactKind::Story => 1.3,
            FactKind::Qa => 1.1,
        }
    }

    /// 拼接进 prompt 时的小节标题。空字符串表示不加标题（`Basics` 直接是一行）。
    fn section(self) -> &'static str {
        match self {
            FactKind::Basics => "",
            FactKind::Skill => "技能",
            FactKind::Project => "项目",
            FactKind::Story => "素材",
            FactKind::Qa => "问答",
        }
    }
}

/// 一条可检索的事实。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub kind: FactKind,
    /// 紧凑正文，一行。整理阶段就压好，运行时不再改写。
    pub text: String,
    /// 检索标签：项目代号、技术名、主题词。小写、去重、不含空白。
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Fact {
    pub fn new(kind: FactKind, text: impl Into<String>, tags: Vec<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            tags,
        }
    }

    /// 字符数（中文按字算）。预算按这个口径控制，不用 token 估算——
    /// 估算误差会让预算形同虚设。
    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }
}

/// 整份档案。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// 整理时间（毫秒时间戳）。`0` 表示还没整理过。
    #[serde(default)]
    pub organized_at: i64,
    /// 整理时用到的源文档名（仅用于界面展示「基于哪几份文档」）。
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub facts: Vec<Fact>,
}

/// 常驻部分的字符预算：基本信息 + 技能。
pub const ALWAYS_CHAR_BUDGET: usize = 600;

/// 检索部分的字符预算：项目 / 故事 / 问答。
pub const RETRIEVED_CHAR_BUDGET: usize = 2_000;

/// 检索结果：拼好的两段文本，以及本次用到了哪些事实（便于界面/日志核对）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Retrieval {
    /// 常驻段（基本信息 + 技能）。没有内容时为空串。
    pub always: String,
    /// 命中段（项目 / 故事 / 问答）。
    pub matched: String,
    /// 命中的事实下标（对应 [`Profile::facts`]），便于测试与调试。
    pub matched_indices: Vec<usize>,
}

impl Retrieval {
    /// 拼接成可直接塞进 system prompt 的文本。两段都为空时返回空串，
    /// 调用方据此完全不追加（与「没有档案」的行为一致）。
    pub fn to_prompt(&self) -> String {
        let mut out = String::new();
        if !self.always.is_empty() {
            out.push_str("\n候选人档案：\n");
            out.push_str(&self.always);
            out.push('\n');
        }
        if !self.matched.is_empty() {
            out.push_str("\n相关经历：\n");
            out.push_str(&self.matched);
            out.push('\n');
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.always.is_empty() && self.matched.is_empty()
    }
}

impl Profile {
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// 按问题检索出本次要发送的事实。
    ///
    /// 打分规则（全部本地字符串运算，零延迟）：
    /// - 事实的 `tags` 或正文里出现的查询词越多，分越高；
    /// - 标签命中比正文命中权重高（标签是整理阶段刻意提炼的）；
    /// - 乘上 [`FactKind::weight`]。
    ///
    /// 预算控制：
    /// - 常驻段（`Basics`/`Skill`）优先，超预算就按原顺序截断——
    ///   基本信息最重要，宁可少一条技能也不能丢姓名年限；
    /// - 检索段按分数从高到低填，装不下就停（不做半截事实，避免句子断在中间）。
    pub fn retrieve(&self, question: &str) -> Retrieval {
        let query = tokenize(question);

        let mut result = Retrieval::default();

        // —— 常驻段 ——
        let mut always_used = 0usize;
        for fact in self.facts.iter().filter(|f| f.kind.is_always_included()) {
            let cost = fact.char_len() + 1;
            if always_used + cost > ALWAYS_CHAR_BUDGET {
                // 基本信息还没出现时不能停：先保证 Basics 至少有一条。
                if !result.always.is_empty() {
                    continue;
                }
            }
            push_line(&mut result.always, fact, &mut always_used);
        }

        // —— 检索段 ——
        if query.is_empty() {
            // 没有问题词（比如纯标点）：不带检索段，省 token。
            return result;
        }

        let mut scored: Vec<(usize, f32)> = self
            .facts
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.kind.is_always_included())
            .filter_map(|(index, fact)| {
                let score = score_fact(fact, &query);
                if score > 0.0 {
                    Some((index, score))
                } else {
                    None
                }
            })
            .collect();

        // 稳定排序：分数相同时保持原顺序（整理时的顺序有意义）。
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut used = 0usize;
        for (index, _) in scored {
            let fact = &self.facts[index];
            let cost = fact.char_len() + 1;
            if used + cost > RETRIEVED_CHAR_BUDGET {
                // 装不下就停：跳过这条继续试更短的，可能还能塞一条小的。
                continue;
            }
            push_line(&mut result.matched, fact, &mut used);
            result.matched_indices.push(index);
        }

        result
    }
}

/// 追加一行事实；`Basics` 不加小节标题，其余类型首次出现时加。
fn push_line(out: &mut String, fact: &Fact, used: &mut usize) {
    let header = fact.kind.section();
    if !header.is_empty() && !out.contains(&format!("[{header}]")) {
        out.push_str(&format!("[{header}]\n"));
        *used += header.chars().count() + 3;
    }
    out.push_str(&fact.text);
    out.push('\n');
    *used += fact.char_len() + 1;
}

/// 给一条事实打分。
fn score_fact(fact: &Fact, query: &[String]) -> f32 {
    let text_lower = fact.text.to_lowercase();
    let mut score = 0.0f32;
    for word in query {
        if fact.tags.iter().any(|tag| tag.to_lowercase() == *word) {
            // 标签精确命中：整理阶段刻意提炼过，信号最强。
            score += 3.0;
        } else if fact
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(word.as_str()))
        {
            score += 2.0;
        } else if text_lower.contains(word.as_str()) {
            score += 1.0;
        }
    }
    score * fact.kind.weight()
}

/// 把问题切成检索词。
///
/// 中文没有空格，按「二元组」切分（相邻两个字一组）——这是无词典中文检索的
/// 常规做法，能匹配「自动化」「性能」这类词，也不会因为整句匹配而漏掉。
/// 英文/数字按非字母数字切分后整词保留。
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut ascii_run = String::new();
    let mut cjk: Vec<char> = Vec::new();

    let flush_ascii = |run: &mut String, out: &mut Vec<String>| {
        if run.chars().count() >= 2 {
            out.push(run.to_lowercase());
        }
        run.clear();
    };

    let flush_cjk = |buf: &mut Vec<char>, out: &mut Vec<String>| {
        if buf.len() == 1 {
            // 单个汉字信息量太低（「的」「了」），丢掉。
            buf.clear();
            return;
        }
        for pair in buf.windows(2) {
            out.push(pair.iter().collect::<String>());
        }
        buf.clear();
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, &mut tokens);
            ascii_run.push(ch);
        } else if is_cjk(ch) {
            flush_ascii(&mut ascii_run, &mut tokens);
            cjk.push(ch);
        } else {
            flush_ascii(&mut ascii_run, &mut tokens);
            flush_cjk(&mut cjk, &mut tokens);
        }
    }
    flush_ascii(&mut ascii_run, &mut tokens);
    flush_cjk(&mut cjk, &mut tokens);

    // 去重但保序。
    let mut seen = std::collections::HashSet::new();
    tokens.retain(|t| seen.insert(t.clone()));
    tokens
}

/// 是否 CJK 统一表意文字（覆盖常用汉字区）。
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        Profile {
            organized_at: 1,
            sources: vec!["简历.md".into(), "项目A.md".into()],
            facts: vec![
                Fact::new(FactKind::Basics, "张三 | 5年测试开发 | 本科", vec![]),
                Fact::new(
                    FactKind::Skill,
                    "Playwright:熟练 | Python:熟练",
                    vec!["playwright".into()],
                ),
                Fact::new(
                    FactKind::Project,
                    "[P1] 广发银行自动化平台 | Playwright+Pytest | 回归3天→4小时",
                    vec!["p1".into(), "playwright".into(), "自动化".into()],
                ),
                Fact::new(
                    FactKind::Project,
                    "[P2] 性能压测平台 | JMeter | 支撑双十一 10 万 QPS",
                    vec!["p2".into(), "性能".into(), "jmeter".into()],
                ),
                Fact::new(
                    FactKind::Story,
                    "[S1] 最难的问题：P1 多环境数据隔离 → fixture 工厂 + 事务回滚",
                    vec!["p1".into(), "难点".into()],
                ),
                Fact::new(
                    FactKind::Qa,
                    "[Q1] 离职原因：想做更偏工程化的质量建设",
                    vec!["离职".into()],
                ),
            ],
        }
    }

    #[test]
    fn empty_profile_retrieves_nothing() {
        let profile = Profile::default();
        let r = profile.retrieve("你做过什么自动化？");
        assert!(r.is_empty());
        assert_eq!(r.to_prompt(), "");
    }

    #[test]
    fn basics_and_skills_are_always_included() {
        let r = sample().retrieve("今天天气不错");
        assert!(r.always.contains("张三"));
        assert!(r.always.contains("Playwright"));
        // 没命中任何项目/故事/问答。
        assert!(r.matched.is_empty());
    }

    #[test]
    fn project_is_retrieved_by_topic() {
        let r = sample().retrieve("你们自动化是怎么做的？");
        assert!(
            r.matched.contains("广发银行"),
            "自动化问题应命中 P1：{}",
            r.matched
        );
        // 性能项目不该被这条问题带出来。
        assert!(!r.matched.contains("10 万 QPS"));
    }

    #[test]
    fn performance_question_retrieves_performance_project() {
        let r = sample().retrieve("做过性能测试吗？");
        assert!(r.matched.contains("性能压测"), "应命中 P2：{}", r.matched);
        assert!(!r.matched.contains("广发银行"));
    }

    #[test]
    fn resignation_question_retrieves_qa() {
        let r = sample().retrieve("为什么离职？");
        assert!(r.matched.contains("离职原因"), "{}", r.matched);
    }

    #[test]
    fn difficulty_question_retrieves_story() {
        let r = sample().retrieve("讲一个你遇到的技术难点");
        assert!(r.matched.contains("最难的问题"), "{}", r.matched);
    }

    #[test]
    fn english_question_matches_english_tags() {
        let r = sample().retrieve("How do you use Playwright?");
        assert!(r.matched.contains("Playwright"), "{}", r.matched);
    }

    #[test]
    fn retrieved_section_respects_budget() {
        // 造 20 条各 300 字、都命中同一关键词的项目。
        let mut profile = Profile::default();
        for i in 0..20 {
            profile.facts.push(Fact::new(
                FactKind::Project,
                format!("[P{i}] 自动化 {}", "细节".repeat(150)),
                vec!["自动化".into()],
            ));
        }
        let r = profile.retrieve("自动化怎么做");
        assert!(
            r.matched.chars().count() <= RETRIEVED_CHAR_BUDGET + 40,
            "检索段超预算：{} 字",
            r.matched.chars().count()
        );
        assert!(!r.matched_indices.is_empty(), "至少要带一条");
    }

    #[test]
    fn always_section_respects_budget_but_keeps_basics() {
        let mut profile = Profile::default();
        profile
            .facts
            .push(Fact::new(FactKind::Basics, "张三 | 5年", vec![]));
        for i in 0..30 {
            profile.facts.push(Fact::new(
                FactKind::Skill,
                format!("技能{i}:{}", "很熟练".repeat(30)),
                vec![],
            ));
        }
        let r = profile.retrieve("随便问");
        assert!(
            r.always.chars().count() <= ALWAYS_CHAR_BUDGET + 40,
            "常驻段超预算：{} 字",
            r.always.chars().count()
        );
        assert!(r.always.contains("张三"), "基本信息不能被技能挤掉");
    }

    #[test]
    fn tokenize_cjk_uses_bigrams() {
        let t = tokenize("自动化测试");
        assert!(t.contains(&"自动".to_string()));
        assert!(t.contains(&"动化".to_string()));
        // 单字不产出。
        assert!(!t.contains(&"测".to_string()));
    }

    #[test]
    fn tokenize_keeps_english_words() {
        let t = tokenize("用 Playwright 做 UI 自动化");
        assert!(t.contains(&"playwright".to_string()));
        // 单字母 "UI" 只有 2 字符，按规则保留。
        assert!(t.contains(&"ui".to_string()));
    }

    #[test]
    fn tokenize_empty_and_punctuation_only() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("，。！？").is_empty());
    }

    #[test]
    fn prompt_has_section_headers() {
        let r = sample().retrieve("自动化");
        let prompt = r.to_prompt();
        assert!(prompt.contains("候选人档案"));
        assert!(prompt.contains("相关经历"));
    }
}
