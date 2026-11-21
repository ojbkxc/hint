import { useState } from "react";
import type { FormAction, Provider, Settings } from "./types";
import { PRESETS, detectProvider, normalizeBaseUrl } from "./types";
import { deriveStatusTone, statusToneClass } from "./statusTone";

type Props = {
  form: Settings;
  apiKey: string;
  keySaved: boolean;
  busy: boolean;
  action: FormAction;
  answer: string;
  status: string;
  /** 接口返回的可用模型列表（点「刷新模型」拉取）。 */
  models: string[];
  onApiKey: (value: string) => void;
  onPatch: (next: Partial<Settings>) => void;
  onTest: () => Promise<{ text: string; model: string } | null>;
  onLoadModels: () => void;
};

export default function ModelCard(props: Props) {
  const { form, apiKey, keySaved, busy, action, answer, status } = props;
  const [visionHint, setVisionHint] = useState(false);

  return (
    <section className="card">
      <div className="card-head">
        <h2>模型接口</h2>
      </div>
      <div className="card-body">
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
        <p className="muted small">
          Base URL 只填域名也行，会自动补全：OpenAI 兼容补 `/v1`，Anthropic 和 Gemini
          自动补版本段；误贴完整端点（含 `/chat/completions` 等）也会自动清理。
          粘贴 Anthropic / Gemini 官方域名时会自动切换协议。
        </p>

        <div className="field">
          <label className="field-label" htmlFor="model-provider">
            接口协议
          </label>
          <select
            id="model-provider"
            className="form-input"
            value={form.provider}
            onChange={(event) =>
              props.onPatch({ provider: event.target.value as Provider })
            }
          >
            <option value="openai">OpenAI 兼容（DeepSeek / 通义 / 智谱 / Ollama）</option>
            <option value="anthropic">Anthropic Messages</option>
            <option value="gemini">Gemini generateContent</option>
          </select>
        </div>

        <div className="field">
          <label className="field-label" htmlFor="model-base-url">
            Base URL
          </label>
          <input
            id="model-base-url"
            className="form-input"
            value={form.baseUrl}
            onChange={(event) => props.onPatch({ baseUrl: event.target.value })}
            onBlur={() => {
              // 先按域名识别协议，再用识别到的协议规范化 URL：
              // 这样粘贴 Anthropic / Gemini / 常见 OpenAI 兼容域名时，
              // 协议会自动切过去，版本段也按对应协议补全。
              const detected = detectProvider(form.baseUrl);
              const provider = detected ?? form.provider;
              const baseUrl = normalizeBaseUrl(form.baseUrl, provider);
              if (baseUrl === form.baseUrl && provider === form.provider) return;
              props.onPatch({
                baseUrl,
                ...(provider !== form.provider ? { provider } : {}),
              });
            }}
            placeholder="https://api.deepseek.com/v1（只填域名也行）"
            spellCheck={false}
          />
        </div>

        <div className="field">
          <label className="field-label" htmlFor="model-name">
            模型名
          </label>
          <div className="inline-row">
            <input
              id="model-name"
              className="form-input"
              list="model-options"
              value={form.model}
              onChange={(event) => props.onPatch({ model: event.target.value })}
              placeholder="点「刷新模型」从接口拉取"
              spellCheck={false}
            />
            <datalist id="model-options">
              {props.models.map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
            <button
              type="button"
              className="secondary"
              onClick={() => void props.onLoadModels()}
              disabled={busy}
            >
              {action === "models" ? "拉取中…" : "刷新模型"}
            </button>
          </div>
        </div>

        <div className="field">
          <label className="field-label" htmlFor="model-api-key">
            API Key
          </label>
          <input
            id="model-api-key"
            className="form-input"
            type="password"
            value={apiKey}
            onChange={(event) => props.onApiKey(event.target.value)}
            placeholder={keySaved ? "已保存，留空则不修改" : "粘贴 API Key"}
            spellCheck={false}
          />
        </div>
        <p className="muted small">
          填好后点页面底部「保存全部配置」一并保存；点「测试连接」会自动保存。
        </p>

        <div className="action-row">
          <button type="button" onClick={() => void props.onTest()} disabled={busy}>
            {action === "test" ? "测试中…" : "测试连接"}
          </button>
        </div>

        {answer && (
          <>
            <span className="visually-hidden">测试连接返回内容</span>
            <pre aria-label="测试连接返回内容" className="answer-box">
              {answer}
            </pre>
          </>
        )}
        {status && <p className={statusToneClass(deriveStatusTone(status))}>{status}</p>}

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
      </div>
    </section>
  );
}
