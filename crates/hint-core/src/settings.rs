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
    /// 麦克风设备 ID；空串表示系统默认输入设备。
    #[serde(default)]
    pub mic_device: String,
    /// 系统声输出设备 ID；空串表示系统默认播放设备。
    #[serde(default)]
    pub output_device: String,
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
            mic_device: String::new(),
            output_device: String::new(),
        }
    }
}

impl Settings {
    /// 读取配置；文件不存在时回退默认值。
    ///
    /// 解析失败说明文件已损坏（例如上次写入中途断电）：把坏文件重命名为
    /// `settings.json.bad` 留证，再回退默认值，避免下次保存时静默覆盖用户资料。
    pub fn load(path: &Path) -> Self {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(_) => return Self::default(),
        };
        match serde_json::from_str(&raw) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "配置文件损坏，已备份并回退默认值");
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                Self::default()
            }
        }
    }

    /// 原子写入：先写临时文件，再替换目标，避免中途崩溃留下半截 JSON。
    ///
    /// Windows 的 `rename` 不能覆盖已存在的目标，因此失败时先删除再重试；
    /// 这段时间窗口极小，且临时文件内容始终完整。
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(self)?)?;
        match std::fs::rename(&temp, path) {
            Ok(()) => Ok(()),
            Err(_) => {
                let _ = std::fs::remove_file(path);
                std::fs::rename(&temp, path)?;
                Ok(())
            }
        }
    }

    /// 组装系统提示词。简历与项目资料为空时不注入对应段落。
    pub fn system_prompt(&self) -> String {
        let style = match self.answer_style {
            AnswerStyle::Concise => {
                "用候选人第一人称口语化回答：先给结论，再给 2-4 个要点，总长不超过 150 字，不要复述问题。"
            }
            AnswerStyle::Outline => "用候选人第一人称给出 5-7 条要点提纲，每条一行，不要展开长篇。",
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
