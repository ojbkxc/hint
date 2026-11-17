import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Provider = "openai" | "anthropic" | "gemini";
type AnswerStyle = "concise" | "outline";
type TriggerMode = "auto" | "manual";

type Settings = {
  provider: Provider;
  baseUrl: string;
  model: string;
  resume: string;
  projects: string;
  answerStyle: AnswerStyle;
  triggerMode: TriggerMode;
};

type ChatResponse = { text: string; model: string };

const PRESETS: Array<{
  label: string;
  provider: Provider;
  baseUrl: string;
  model: string;
}> = [
  {
    label: "DeepSeek",
    provider: "openai",
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
  },
  {
    label: "OpenAI",
    provider: "openai",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
  },
  {
    label: "通义千问",
    provider: "openai",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus",
  },
  {
    label: "Claude",
    provider: "anthropic",
    baseUrl: "https://api.anthropic.com",
    model: "claude-sonnet-5-5",
  },
  {
    label: "Gemini",
    provider: "gemini",
    baseUrl: "https://generativelanguage.googleapis.com",
    model: "gemini-2.5-flash",
  },
];

const EMPTY: Settings = {
  provider: "openai",
  baseUrl: "",
  model: "",
  resume: "",
  projects: "",
  answerStyle: "concise",
  triggerMode: "auto",
};

export default function Dashboard() {
  const [form, setForm] = useState<Settings>(EMPTY);
  const [apiKey, setApiKey] = useState("");
  const [keySaved, setKeySaved] = useState(false);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState("");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        const settings = await invoke<Settings>("get_settings");
        setForm(settings);
        const saved = await invoke<boolean>("has_api_key", {
          provider: settings.provider,
        });
        setKeySaved(saved);
      } catch (error) {
        setStatus(`读取配置失败：${String(error)}`);
      } finally {
        setLoaded(true);
      }
    })();
  }, []);

  function patch(next: Partial<Settings>) {
    setForm((current) => ({ ...current, ...next }));
  }

  async function saveSettings() {
    setBusy(true);
    setStatus("");
    try {
      await invoke("save_settings", { settings: form });
      setStatus("配置已保存");
    } catch (error) {
      setStatus(`保存失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function saveKey() {
    if (!apiKey.trim()) {
      setStatus("请先填写 API Key");
      return;
    }
    setBusy(true);
    setStatus("");
    try {
      await invoke("save_api_key", { provider: form.provider, key: apiKey.trim() });
      setKeySaved(true);
      setApiKey("");
      setStatus("API Key 已写入 Windows 凭据管理器");
    } catch (error) {
      setStatus(`保存 Key 失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function testConnection() {
    setBusy(true);
    setStatus("");
    setAnswer("");
    try {
      const response = await invoke<ChatResponse>("test_llm", {
        prompt: "请用一句话介绍你自己，并确认你能正常工作。",
      });
      setAnswer(response.text);
      setStatus(`连接成功，模型：${response.model}`);
    } catch (error) {
      setStatus(`连接失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="dashboard-root">
      <header className="dashboard-header">
        <div>
          <h1>hint</h1>
          <p className="subtitle">面试实时提词器 · M0 骨架</p>
        </div>
        <div className="hint-badge">Ctrl+Shift+Space 显示浮层</div>
      </header>

      {!loaded ? (
        <p className="muted">正在读取配置…</p>
      ) : (
        <div className="dashboard-grid">
          <section className="card">
            <h2>模型接口</h2>
            <div className="preset-row">
              {PRESETS.map((preset) => (
                <button
                  key={preset.label}
                  type="button"
                  className="ghost"
                  onClick={() =>
                    setForm((current) => ({
                      ...current,
                      provider: preset.provider,
                      baseUrl: preset.baseUrl,
                      model: preset.model,
                    }))
                  }
                >
                  {preset.label}
                </button>
              ))}
            </div>

            <label>
              接口协议
              <select
                value={form.provider}
                onChange={(event) => patch({ provider: event.target.value as Provider })}
              >
                <option value="openai">OpenAI 兼容（DeepSeek / 通义 / 智谱 / Ollama）</option>
                <option value="anthropic">Anthropic Messages</option>
                <option value="gemini">Gemini generateContent</option>
              </select>
            </label>

            <label>
              Base URL
              <input
                value={form.baseUrl}
                onChange={(event) => patch({ baseUrl: event.target.value })}
                placeholder="https://api.deepseek.com/v1"
                spellCheck={false}
              />
            </label>

            <label>
              模型名
              <input
                value={form.model}
                onChange={(event) => patch({ model: event.target.value })}
                placeholder="deepseek-chat"
                spellCheck={false}
              />
            </label>

            <label>
              API Key
              <div className="inline-row">
                <input
                  type="password"
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  placeholder={keySaved ? "已保存，留空则不修改" : "粘贴 API Key"}
                  spellCheck={false}
                />
                <button type="button" onClick={saveKey} disabled={busy}>
                  保存
                </button>
              </div>
            </label>

            <div className="action-row">
              <button type="button" onClick={testConnection} disabled={busy}>
                {busy ? "请求中…" : "测试连接"}
              </button>
              <button type="button" className="primary" onClick={saveSettings} disabled={busy}>
                保存配置
              </button>
            </div>

            {answer && <pre className="answer-box">{answer}</pre>}
            {status && <p className="status">{status}</p>}
          </section>

          <section className="card">
            <h2>资料注入</h2>
            <label>
              简历
              <textarea
                rows={9}
                value={form.resume}
                onChange={(event) => patch({ resume: event.target.value })}
                placeholder="粘贴简历文本。M0 阶段直接全文注入，后续会加 token 预算与裁剪。"
              />
            </label>
            <label>
              项目资料
              <textarea
                rows={7}
                value={form.projects}
                onChange={(event) => patch({ projects: event.target.value })}
                placeholder="项目背景、技术栈、你负责的部分、难点与量化结果。"
              />
            </label>
            <p className="muted small">
              资料只存在本机，不会上传到除你所选模型接口以外的任何地方。
            </p>
          </section>

          <section className="card">
            <h2>回答行为</h2>
            <label>
              答案风格
              <select
                value={form.answerStyle}
                onChange={(event) =>
                  patch({ answerStyle: event.target.value as AnswerStyle })
                }
              >
                <option value="concise">口语短答（可直接照念）</option>
                <option value="outline">要点提纲</option>
              </select>
            </label>
            <label>
              触发方式
              <select
                value={form.triggerMode}
                onChange={(event) =>
                  patch({ triggerMode: event.target.value as TriggerMode })
                }
              >
                <option value="auto">自动（检测到问题就生成）</option>
                <option value="manual">按键触发（Ctrl+Shift+A）</option>
              </select>
            </label>
            <p className="muted small">
              这两个选项在 M4 接入语音链路后生效，当前仅保存配置。
            </p>
          </section>
        </div>
      )}
    </div>
  );
}
