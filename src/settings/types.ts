export type Provider = "openai" | "anthropic" | "gemini";
export type AnswerStyle = "concise" | "outline";
export type TriggerMode = "auto" | "manual";

/** 设置页当前正在进行的操作，用于给对应按钮显示准确的阶段文案。 */
export type FormAction = "" | "save" | "test" | "models";

export type Settings = {
  provider: Provider;
  baseUrl: string;
  model: string;
  resume: string;
  projects: string;
  answerStyle: AnswerStyle;
  triggerMode: TriggerMode;
  /** 后端尚未实现时保留空字符串。 */
  micDevice: string;
  outputDevice: string;
  /** 浮层背景不透明度，0.2–1.0（默认 0.88）。文字始终不透明。 */
  overlayOpacity: number;
  /** 浮层字号缩放，0.8–1.6（默认 1.0）。答案、转录、问题行按同一比例放大。 */
  overlayFontScale: number;
};

export type ChatResponse = { text: string; model: string };

export type AudioDevice = { id: string; name: string };
export type AudioDevices = { inputs: AudioDevice[]; outputs: AudioDevice[] };

export const PRESETS: Array<{
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

export const DEFAULT_OVERLAY_OPACITY = 0.88;
export const MIN_OVERLAY_OPACITY = 0.2;
export const MAX_OVERLAY_OPACITY = 1;

/** 浮层字号缩放的默认值与上下限（1.0 = 现状大小）。 */
export const DEFAULT_OVERLAY_FONT_SCALE = 1;
export const MIN_OVERLAY_FONT_SCALE = 0.8;
export const MAX_OVERLAY_FONT_SCALE = 1.6;

/**
 * Base URL 规范化：把用户常见的各种写法统一成后端能直接拼接的形态。
 *
 * - 缺协议头时补 `https://`；
 * - 误贴完整端点（`/chat/completions`、`/messages`、`:generateContent`）时剥掉；
 * - OpenAI 兼容协议在没有版本段时补 `/v1`；
 * - Anthropic / Gemini 保持域名（后端会自动补 `/v1`、`/v1beta`）。
 */
export function normalizeBaseUrl(raw: string, provider: Provider): string {
  let url = raw.trim();
  if (!url) return "";
  if (!/^https?:\/\//i.test(url)) url = `https://${url}`;
  url = url.replace(/\/+$/, "");
  url = url
    .replace(/\/chat\/completions$/i, "")
    .replace(/\/messages$/i, "")
    .replace(/:(stream)?generateContent$/i, "")
    .replace(/\/models\/[^/]*$/i, "");
  url = url.replace(/\/+$/, "");
  if (!url) return "";
  if (provider === "openai" && !hasVersionSuffix(url)) {
    return `${url}/v1`;
  }
  return url;
}

/** 路径最后一段是否形如 v1 / v2 / v1beta 的版本段。 */
function hasVersionSuffix(url: string): boolean {
  const last = url.split("/").pop() ?? "";
  return /^v\d/i.test(last);
}

/**
 * 根据域名自动识别协议；识别不出来时返回 null（保持用户当前选择）。
 * 只识别特征明确的域名，避免把自建网关误判成其他协议。
 */
export function detectProvider(raw: string): Provider | null {
  const url = raw.trim();
  if (!url) return null;
  let host = "";
  try {
    host = new URL(/^https?:\/\//i.test(url) ? url : `https://${url}`).hostname;
  } catch {
    return null;
  }
  const hostname = host.toLowerCase();
  if (matchesHost(hostname, "anthropic.com")) return "anthropic";
  if (matchesHost(hostname, "generativelanguage.googleapis.com")) return "gemini";
  const openaiHosts = [
    "openai.com",
    "deepseek.com",
    "dashscope.aliyuncs.com",
    "bigmodel.cn",
    "moonshot.cn",
    "siliconflow.cn",
    "localhost",
    "127.0.0.1",
  ];
  if (openaiHosts.some((domain) => matchesHost(hostname, domain))) return "openai";
  return null;
}

function matchesHost(hostname: string, domain: string): boolean {
  return hostname === domain || hostname.endsWith(`.${domain}`);
}

/** 不透明度统一 clamp；非数字或越界值一律回落默认值。 */
export function normalizeOpacity(raw: unknown): number {
  const parsed =
    typeof raw === "number"
      ? raw
      : typeof raw === "string" && raw.trim() !== ""
        ? Number(raw)
        : Number.NaN;
  if (!Number.isFinite(parsed)) return DEFAULT_OVERLAY_OPACITY;
  return Math.min(MAX_OVERLAY_OPACITY, Math.max(MIN_OVERLAY_OPACITY, parsed));
}

/** 浮层字号缩放归一：非法值回落 1.0，越界钳到 0.8–1.6。 */
export function normalizeFontScale(raw: unknown): number {
  const parsed =
    typeof raw === "number"
      ? raw
      : typeof raw === "string" && raw.trim() !== ""
        ? Number(raw)
        : Number.NaN;
  if (!Number.isFinite(parsed)) return DEFAULT_OVERLAY_FONT_SCALE;
  return Math.min(
    MAX_OVERLAY_FONT_SCALE,
    Math.max(MIN_OVERLAY_FONT_SCALE, parsed),
  );
}

export const EMPTY_SETTINGS: Settings = {
  provider: "openai",
  baseUrl: "",
  model: "",
  resume: "",
  projects: "",
  answerStyle: "concise",
  triggerMode: "auto",
  micDevice: "",
  outputDevice: "",
  overlayOpacity: DEFAULT_OVERLAY_OPACITY,
  overlayFontScale: DEFAULT_OVERLAY_FONT_SCALE,
};

/** 后端返回的 settings 可能还是 M0 形状，缺设备字段时补空。 */
export function normalizeSettings(raw: unknown): Settings {
  if (typeof raw !== "object" || raw === null) return { ...EMPTY_SETTINGS };
  const value = raw as Record<string, unknown>;
  const str = (key: string) =>
    typeof value[key] === "string" ? (value[key] as string) : "";
  const provider =
    value.provider === "anthropic" || value.provider === "gemini"
      ? value.provider
      : "openai";
  return {
    provider,
    baseUrl: str("baseUrl"),
    model: str("model"),
    resume: str("resume"),
    projects: str("projects"),
    answerStyle: value.answerStyle === "outline" ? "outline" : "concise",
    triggerMode: value.triggerMode === "manual" ? "manual" : "auto",
    micDevice: str("micDevice"),
    outputDevice: str("outputDevice"),
    overlayOpacity: normalizeOpacity(
      value.overlayOpacity ?? value.overlay_opacity,
    ),
    overlayFontScale: normalizeFontScale(
      value.overlayFontScale ?? value.overlay_font_scale,
    ),
  };
}

/** list_audio_devices 的返回值可能是 {inputs,outputs} 或纯数组，统一成数组。 */
export function normalizeDevices(
  raw: unknown,
  kind: "inputs" | "outputs",
): AudioDevice[] {
  if (Array.isArray(raw)) return normalizeDeviceList(raw);
  if (typeof raw !== "object" || raw === null) return [];
  return normalizeDeviceList((raw as Record<string, unknown>)[kind]);
}

function normalizeDeviceList(raw: unknown): AudioDevice[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((item, index) => {
      if (typeof item === "string") return { id: item, name: item };
      if (typeof item === "object" && item !== null) {
        const value = item as Record<string, unknown>;
        const id =
          typeof value.id === "string"
            ? value.id
            : typeof value.deviceId === "string"
              ? value.deviceId
              : "";
        const name =
          typeof value.name === "string"
            ? value.name
            : typeof value.label === "string"
              ? value.label
              : id;
        return { id: id || `device-${index}`, name: name || `设备 ${index + 1}` };
      }
      return { id: `device-${index}`, name: `设备 ${index + 1}` };
    })
    .filter((device) => device.id !== "");
}