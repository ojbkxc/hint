export type Provider = "openai" | "anthropic" | "gemini";
export type AnswerStyle = "concise" | "outline";
export type TriggerMode = "auto" | "manual";

/** 出网代理模式：直连 / 跟随系统 / 手填。 */
export type ProxyMode = "direct" | "system" | "manual";
export const PROXY_MODES: ProxyMode[] = ["direct", "system", "manual"];

/**
 * 语音识别语言提示——**兼容字段，已不对用户暴露**。
 *
 * 实测该参数对 SenseVoice 基本无效：同一段中文音频，auto/zh/en/ja/ko/yue
 * 六个值输出逐字相同，强制 ja/ko 也照样输出中文。所以设置页那个下拉框已移除，
 * 这里保留类型只为让配置读写不丢字段（后端字段也在）。
 */
export type SttLanguage = "zh" | "en" | "ja" | "ko" | "yue" | "auto";
export const STT_LANGUAGES: SttLanguage[] = ["zh", "en", "ja", "ko", "yue", "auto"];
export const DEFAULT_STT_LANGUAGE: SttLanguage = "zh";

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
  /** 浮层字号缩放，0.8–3（默认 1.0）。只缩放答案（含历史答案），转录与问题行固定大小。 */
  overlayFontScale: number;
  /**
   * 浮层是否从屏幕捕获中排除（**默认 false**）。
   *
   * 开启后部分会议软件共享屏幕时会显示成黑块（比被看到更尴尬），还有一类
   * 捕获方式完全拦不住，所以默认关；需要时由用户在设置页自行开启并先自测。
   */
  overlayContentProtected: boolean;
  /** 出网代理模式。装代理软件的用户要靠它连模型接口。 */
  proxyMode: ProxyMode;
  /** 手填代理地址（仅 manual 生效），如 http://127.0.0.1:7890。 */
  proxyUrl: string;
  /** 语音识别语言提示（默认 zh）。 */
  sttLanguage: SttLanguage;
  /**
   * 配置结构版本（后端迁移用）。
   *
   * **必须原样透传**：后端靠它判断要不要跑一次性的旧值修正。前端若把它丢掉，
   * 保存后版本变 0，下次加载会重新迁移一遍——用户手动打开的内容保护会被
   * 强行关掉（真机验证时发现过这个连带问题）。
   */
  schemaVersion: number;
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
export const MAX_OVERLAY_FONT_SCALE = 3;

/**
 * 浮层内容保护默认**关**。
 *
 * 真机实测三条捕获路径行为完全不同：GDI 下正确排除；PrintWindow+
 * PW_RENDERFULLCONTENT 下变成**纯黑块**（腾讯会议类软件共享屏幕时看到的就是
 * 这个黑框）；旧式 PrintWindow 完全绕过保护、内容原样可见。
 * 开了反而暴露「在用工具」，所以默认关；需要时用户自己开并先在本地试过。
 */
export const DEFAULT_OVERLAY_CONTENT_PROTECTED = false;

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

/** 浮层字号缩放归一：非法值回落 1.0，越界钳到 0.8–3。 */
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
  overlayContentProtected: DEFAULT_OVERLAY_CONTENT_PROTECTED,
  proxyMode: "system",
  proxyUrl: "",
  sttLanguage: DEFAULT_STT_LANGUAGE,
  schemaVersion: 1,
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
    overlayContentProtected:
      (value.overlayContentProtected ?? value.overlay_content_protected) !== false,
    // 默认跟随系统：装了代理软件的机器不用手动配就能连上。
    // 只有显式 "direct" 才算直连；字段缺失（老配置）= 跟随系统，
    // 而没装代理软件的机器上「跟随系统」探测不到代理，实际等价于直连。
    proxyMode:
      value.proxyMode === "direct" || value.proxyMode === "manual"
        ? value.proxyMode
        : "system",
    proxyUrl: str("proxyUrl"),
    // 白名单校验：后端只接受这几种，前端也不让脏值进表单。
    sttLanguage: (STT_LANGUAGES as string[]).includes(str("sttLanguage"))
      ? (str("sttLanguage") as SttLanguage)
      : DEFAULT_STT_LANGUAGE,
    // 原样透传：丢了这个值，保存后后端会重新迁移一遍。
    schemaVersion:
      typeof value.schemaVersion === "number" ? value.schemaVersion : 1,
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