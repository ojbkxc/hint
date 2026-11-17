use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
}

impl ProviderKind {
    /// 用作 keyring 的账号名，与前端传值保持一致。
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Gemini => "gemini",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnswerStyle {
    Concise,
    Outline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub provider: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub resume: String,
    #[serde(default)]
    pub projects: String,
    pub answer_style: AnswerStyle,
    pub trigger_mode: TriggerMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OpenAi,
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            resume: String::new(),
            projects: String::new(),
            answer_style: AnswerStyle::Concise,
            trigger_mode: TriggerMode::Auto,
        }
    }
}

impl Settings {
    /// 读取配置；文件不存在或损坏时回退到默认值。
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// 组装系统提示词。简历与项目资料为空时不注入对应段落。
    pub fn system_prompt(&self) -> String {
        let style = match self.answer_style {
            AnswerStyle::Concise => {
                "用候选人第一人称口语化回答：先给结论，再给 2-4 个要点，总长不超过 150 字，不要复述问题。"
            }
            AnswerStyle::Outline => {
                "用候选人第一人称给出 5-7 条要点提纲，每条一行，不要展开长篇。"
            }
        };

        let mut prompt = format!(
            "你是面试实时提词器，服务对象是候选人本人。\n输出要求：\n- {style}\n- 不要使用 Markdown 标题，不要客套，不要编造不确定的内容。\n"
        );

        if !self.resume.trim().is_empty() {
            prompt.push_str("\n候选人简历：\n");
            prompt.push_str(self.resume.trim());
            prompt.push('\n');
        }

        if !self.projects.trim().is_empty() {
            prompt.push_str("\n项目资料：\n");
            prompt.push_str(self.projects.trim());
            prompt.push('\n');
        }

        prompt
    }
}
