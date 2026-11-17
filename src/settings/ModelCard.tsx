import { useState } from "react";
import type { Provider, Settings } from "./types";
import { PRESETS } from "./types";

type Props = {
  form: Settings;
  apiKey: string;
  keySaved: boolean;
  busy: boolean;
  answer: string;
  status: string;
  onApiKey: (value: string) => void;
  onSaveKey: () => void;
  onPatch: (next: Partial<Settings>) => void;
  onTest: () => Promise<{ text: string; model: string } | null>;
  onSave: () => void;
};

export default function ModelCard(props: Props) {
  const { form, apiKey, keySaved, busy, answer, status } = props;
  const [visionHint, setVisionHint] = useState(false);

  return (
    <section className="card">
      <h2>模型接口</h2>
      <div className="preset-row">
        {PRESETS.map((preset) => (
          <button
            key={preset.label}
            type="button"
            className="ghost"
            onClick={() =>
              props.onPatch({
                provider: preset.provider,
                baseUrl: preset.baseUrl,
                model: preset.model,
              })
            }
          >
            {preset.label}
          </button>
        ))}
      </div>
      <p className="muted small">
        预设模型名会随厂商更新，可在下方直接修改。
      </p>

      <label>
        接口协议
        <select
          value={form.provider}
          onChange={(event) =>
            props.onPatch({ provider: event.target.value as Provider })
          }
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
          onChange={(event) => props.onPatch({ baseUrl: event.target.value })}
          placeholder="https://api.deepseek.com/v1"
          spellCheck={false}
        />
      </label>

      <label>
        模型名
        <input
          value={form.model}
          onChange={(event) => props.onPatch({ model: event.target.value })}
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
            onChange={(event) => props.onApiKey(event.target.value)}
            placeholder={keySaved ? "已保存，留空则不修改" : "粘贴 API Key"}
            spellCheck={false}
          />
          <button type="button" onClick={props.onSaveKey} disabled={busy}>
            保存
          </button>
        </div>
      </label>

      <div className="action-row">
        <button type="button" onClick={() => void props.onTest()} disabled={busy}>
          {busy ? "请求中…" : "测试连接"}
        </button>
        <button type="button" className="primary" onClick={props.onSave} disabled={busy}>
          保存配置
        </button>
      </div>

      {answer && <pre className="answer-box">{answer}</pre>}
      {status && <p className="status">{status}</p>}

      <div className="screenshot-row">
        <button
          type="button"
          className="ghost m5"
          onClick={() => setVisionHint(true)}
          title="截图问答在 M5 实现，当前版本尚未启用"
        >
          截图问答
          <span className="tag-m5">M5 预留</span>
        </button>
        {visionHint && (
          <p className="muted small">
            截图问答尚未启用：v1 只预留了窗口与接口，需等 M5 接入视觉模型后开放。
            {form.model ? ` 当前模型「${form.model}」是否支持图片需在 M5 由设置项标注。` : ""}
          </p>
        )}
      </div>
    </section>
  );
}