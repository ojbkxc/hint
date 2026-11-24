import { useState } from "react";
import type { FormAction, Provider, Settings } from "./types";
import { PRESETS, detectProvider, normalizeBaseUrl } from "./types";
import type { TestConnectionResult } from "./useSettingsForm";

type Props = {
  form: Settings;
  apiKey: string;
  keySaved: boolean;
  busy: boolean;
  action: FormAction;
  answer: string;
  /** 接口返回的可用模型列表（点「刷新模型」拉取）。 */
  models: string[];
  onApiKey: (value: string) => void;
  onPatch: (next: Partial<Settings>) => void;
  onTest: () => Promise<TestConnectionResult>;
  onLoadModels: () => void;
};

/**
 * 协议分段选项：与原原生 select 的三个 option 一一对应，
 * 值（Provider）与 onPatch 回调语义完全不变，仅把展示换成更清晰的分段控件。
 * option 原文案里的补充说明（DeepSeek / 通义等）移到选中项下方的说明行，信息不丢。
 */
const PROTOCOL_OPTIONS: Array<{
  value: Provider;
  label: string;
  description: string;
}> = [
  {
    value: "openai",
    label: "OpenAI 兼容",
    description: "适用于 DeepSeek / 通义 / 智谱 / Ollama 等接口。",
  },
  {
    value: "anthropic",
    label: "Anthropic",
    description: "Anthropic Messages 协议。",
  },
  {
    value: "gemini",
    label: "Gemini",
    description: "Gemini generateContent 协议。",
  },
];

/** 展示层的测试结果（成功 / 失败 + 耗时 + 模型摘要），只在 tsx 内派生，不回写 hook。 */
type TestOutcome = {
  ok: boolean;
  model: string;
  elapsedMs: number;
  /** 失败原因（成功时为空）。 */
  error: string;
};

function formatElapsed(ms: number): string {
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}

function EyeOpenIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function EyeOffIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
      <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
      <path d="M14.12 14.12a3 3 0 1 1-4.24-4.24" />
      <line x1="1" y1="1" x2="23" y2="23" />
    </svg>
  );
}

export default function ModelCard(props: Props) {
  const { form, apiKey, keySaved, busy, action, answer, models } = props;
  const [visionHint, setVisionHint] = useState(false);
  /** API Key 明文切换：只影响本输入框的 type，Key 内容不进任何日志 / DOM 属性。 */
  const [keyVisible, setKeyVisible] = useState(false);
  /** 最近一次「测试连接」的结果卡数据；null = 尚未测试。 */
  const [testResult, setTestResult] = useState<TestOutcome | null>(null);

  /** 表单与某个预设完全一致时高亮该芯片（纯展示，不影响点击行为）。 */
  const activePreset = PRESETS.find(
    (preset) =>
      preset.provider === form.provider &&
      preset.baseUrl === form.baseUrl &&
      preset.model === form.model,
  );

  const activeProtocol = PROTOCOL_OPTIONS.find(
    (option) => option.value === form.provider,
  );

  // 与原 onClick={() => void props.onTest()} 等价：同一个回调、同样的禁用逻辑，
  // 只是在展示层补一份耗时与成败，用于渲染结果卡。
  async function runTest() {
    setTestResult(null);
    const startedAt = performance.now();
    const response = await props.onTest();
    const elapsedMs = Math.max(0, Math.round(performance.now() - startedAt));
    const failed = !response || !!response.error;
    setTestResult({
      ok: !failed,
      model: response?.model ?? "",
      elapsedMs,
      error: response?.error ?? (response ? "" : "测试没有返回结果"),
    });
  }

  return (
    <section className="settings-block">
      <div className="field-group">
        <p className="field-group-title">当前生效</p>
        {/* 技术值集中一行：等宽 + tabular-nums，改完立刻能核对，不用在输入框之间来回看。 */}
        <div className="settings-kv">
          <span className="settings-kv-key">协议</span>
          <span className="settings-kv-val">{activeProtocol?.label ?? form.provider}</span>
          <span className="settings-kv-key">Base URL</span>
          <span className="settings-kv-val settings-mono">{form.baseUrl || "未填写"}</span>
          <span className="settings-kv-key">模型名</span>
          <span className="settings-kv-val settings-mono">{form.model || "未填写"}</span>
          <span className="settings-kv-key">API Key</span>
          <span className="settings-kv-val">
            <span className={`badge ${keySaved ? "badge-success" : "badge-neutral"}`}>
              {keySaved ? "已保存" : "未保存"}
            </span>
          </span>
        </div>
      </div>

      <div className="field-group">
        <p className="field-group-title">常用预设</p>
        <div className="preset-row" role="group" aria-label="常用预设">
          {PRESETS.map((preset) => (
            <button
              key={preset.label}
              type="button"
              className={`preset${activePreset === preset ? " active" : ""}`}
              aria-pressed={activePreset === preset}
              title={`一键填充：${preset.baseUrl} / ${preset.model}`}
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
      </div>

      <div className="field-group">
        <p className="field-group-title">接口凭据</p>

        <div className="field">
          <span className="field-label" id="model-provider-label">
            接口协议
          </span>
          <div
            className="segment-bar"
            role="group"
            aria-labelledby="model-provider-label"
          >
            {PROTOCOL_OPTIONS.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`segment${form.provider === option.value ? " active" : ""}`}
                aria-pressed={form.provider === option.value}
                onClick={() => props.onPatch({ provider: option.value })}
              >
                {option.label}
              </button>
            ))}
          </div>
          {activeProtocol && <p className="muted small">{activeProtocol.description}</p>}
        </div>

        <div className="field">
          <label className="field-label" htmlFor="model-base-url">
            Base URL
          </label>
          <input
            id="model-base-url"
            className="form-input settings-mono"
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
          <p className="muted small">
            只填域名也行，会自动补全：OpenAI 兼容补 /v1，Anthropic 和 Gemini
            自动补版本段；误贴完整端点（含 /chat/completions 等）也会自动清理。
            粘贴 Anthropic / Gemini 官方域名时会自动切换协议。
          </p>
        </div>

        <div className="field">
          <div className="field-head">
            <label className="field-label" htmlFor="model-name">
              模型名
            </label>
            <span className="muted small" aria-live="polite">
              {models.length > 0
                ? `已拉取 ${models.length} 个可选模型`
                : "未拉取列表，可手动填写"}
            </span>
          </div>
          <div className="inline-row">
            <input
              id="model-name"
              className="form-input settings-mono"
              list="model-options"
              value={form.model}
              onChange={(event) => props.onPatch({ model: event.target.value })}
              placeholder="点「刷新模型」从接口拉取"
              spellCheck={false}
            />
            <datalist id="model-options">
              {models.map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
            <button
              type="button"
              className="secondary"
              onClick={() => void props.onLoadModels()}
              disabled={busy}
            >
              {action === "models" ? (
                <>
                  <span className="loading-dot" aria-hidden="true" />
                  拉取中…
                </>
              ) : (
                "刷新模型"
              )}
            </button>
          </div>
        </div>

        <div className="field">
          <div className="field-head">
            <label className="field-label" htmlFor="model-api-key">
              API Key
            </label>
          </div>
          <div className="input-wrap">
            <input
              id="model-api-key"
              className="form-input"
              type={keyVisible ? "text" : "password"}
              value={apiKey}
              onChange={(event) => props.onApiKey(event.target.value)}
              placeholder={keySaved ? "已保存，留空则不修改" : "粘贴 API Key"}
              spellCheck={false}
            />
            <button
              type="button"
              className="input-toggle"
              onClick={() => setKeyVisible((visible) => !visible)}
              aria-label={keyVisible ? "隐藏 API Key" : "显示 API Key"}
              aria-pressed={keyVisible}
              title={keyVisible ? "隐藏 API Key" : "显示 API Key"}
            >
              {keyVisible ? <EyeOffIcon /> : <EyeOpenIcon />}
            </button>
          </div>
          <p className="muted small">
            填好后点面板底部固定栏的「保存全部配置」一并保存；点「测试连接」会自动保存。
          </p>
        </div>
      </div>

      <div className="field-group">
        <p className="field-group-title">连接测试</p>
        <div className="action-row">
          <button type="button" onClick={() => void runTest()} disabled={busy}>
            {action === "test" ? (
              <>
                <span className="loading-dot" aria-hidden="true" />
                测试中…
              </>
            ) : (
              "测试连接"
            )}
          </button>
        </div>

        {testResult && (
          <div
            className={`test-result${testResult.ok ? " is-ok" : " is-error"}`}
            role="status"
            aria-live="polite"
          >
            <div className="test-result-head">
              <span className={`badge ${testResult.ok ? "badge-success" : "badge-danger"}`}>
                {testResult.ok ? "连接成功" : "连接失败"}
              </span>
              <span className="test-result-meta">
                {`耗时 ${formatElapsed(testResult.elapsedMs)}`}
                {testResult.model ? ` · 模型 ${testResult.model}` : ""}
              </span>
            </div>
            {/* 失败原因就放在结果卡里：Base URL 错 / Key 错 / 网络不通
                三种情况用户要能当场看出来，不用去翻底栏。 */}
            {testResult.error && (
              <p className="status-error test-result-error">{testResult.error}</p>
            )}
            {answer && (
              <>
                <span className="visually-hidden">测试连接返回内容</span>
                <pre aria-label="测试连接返回内容" className="answer-box">
                  {answer}
                </pre>
              </>
            )}
          </div>
        )}
        {/*
          全局状态（保存结果、拉取结果等）统一由面板底部固定栏显示：
          同一份 form.status 在这里再渲染一次，会让用户看到两行一模一样的提示。
          卡片内只保留本卡片自己的反馈（测试连接结果卡 / 截图提示）。
        */}

        <div className="screenshot-row">
          <button
            type="button"
            className="ghost m5"
            onClick={() => setVisionHint(true)}
            title="截图问答需要支持视觉输入的模型；模型不接受图片时接口会报错"
          >
            截图问答
            <span className="tag-m5">需要视觉模型</span>
          </button>
          {visionHint && (
            <p className="muted small">
              截图问答需要支持视觉输入的模型：按 Ctrl+Shift+S 框选屏幕区域，截图随问题一起发给你自己配置的模型接口；
              模型不接受图片时接口会直接报错，换一个支持视觉的模型即可。
              {form.model ? ` 当前模型「${form.model}」是否支持图片由接口决定，报错即需更换。` : ""}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}