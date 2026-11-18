export type Provider = "openai" | "anthropic" | "gemini";
export type AnswerStyle = "concise" | "outline";
export type TriggerMode = "auto" | "manual";

/** 设置页当前正在进行的操作，用于给对应按钮显示准确的阶段文案。 */
export type FormAction = "" | "save" | "key" | "test";

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