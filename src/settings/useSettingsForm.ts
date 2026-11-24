import { useCallback, useEffect, useRef, useState } from "react";
import { safeInvoke } from "../lib/tauri";
import {
  EMPTY_SETTINGS,
  normalizeBaseUrl,
  normalizeDevices,
  normalizeSettings,
  type FormAction,
  type Settings,
} from "./types";

/**
 * 设置表单状态与后端读写。
 * 读取失败时退回默认值并给出中文提示，不阻塞页面渲染。
 */
export function useSettingsForm() {
  const [form, setForm] = useState<Settings>(EMPTY_SETTINGS);
  const [apiKey, setApiKey] = useState("");
  const [keySaved, setKeySaved] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [action, setAction] = useState<FormAction>("");
  const [models, setModels] = useState<string[]>([]);
  // 模型列表请求序号：切换协议/Base URL 后，旧请求的响应必须丢弃。
  const modelsRequest = useRef(0);
  // 当前协议的最新值。保存/测试要等后端几十秒，回来时表单可能已经切到
  // 别的协议；用它可以判断「这次结果还能不能写回 UI」。渲染期赋值保证
  // 同一个事件循环里拿到的就是最新值（effect 会被推迟到下一个任务）。
  const providerRef = useRef<Settings["provider"]>(form.provider);
  providerRef.current = form.provider;

  useEffect(() => {
    let disposed = false;
    void (async () => {
      const settings = await safeInvoke<unknown>("get_settings");
      if (disposed) return;
      if (settings.ok) {
        setForm(normalizeSettings(settings.data));
      } else {
        setStatus(`读取配置失败：${settings.error}。已使用默认值。`);
      }
      if (!disposed) setLoaded(true);
    })();
    return () => {
      disposed = true;
    };
  }, []);

  // 每个 provider 的 Key 是独立的：切换接口后必须重新查询，不能沿用上一个的结果。
  useEffect(() => {
    let disposed = false;
    setKeySaved(false);
    // 输入框里刚粘贴的 Key 属于旧协议，切协议后必须清空，否则会把 A 的
    // Key 存进 B（或拿去测试 B），必然认证失败。
    setApiKey("");
    // 模型列表来自上一个协议的接口，切换协议后不再适用，清空避免选错。
    modelsRequest.current += 1;
    setModels([]);
    void (async () => {
      const saved = await safeInvoke<unknown>("has_api_key", {
        provider: form.provider,
      });
      if (!disposed && saved.ok) setKeySaved(saved.data === true);
    })();
    return () => {
      disposed = true;
    };
  }, [form.provider]);

  // Base URL 变了（切预设或手改域名）后，上一接口拉到的模型名不再可信。
  useEffect(() => {
    modelsRequest.current += 1;
    setModels([]);
  }, [form.baseUrl]);

  const patch = useCallback((next: Partial<Settings>) => {
    setForm((current) => ({ ...current, ...next }));
  }, []);

  async function hasKeyFor(provider: Settings["provider"]): Promise<boolean> {
    const saved = await safeInvoke<unknown>("has_api_key", { provider });
    const present = saved.ok && saved.data === true;
    setKeySaved(present);
    return present;
  }

  async function save() {
    setBusy(true);
    setAction("save");
    setStatus("");
    // 本次保存针对的协议；期间切协议后不能再把 Key/状态写到新协议头上。
    const providerAtStart = form.provider;
    try {
      // 表单里填了 Key 就随「保存全部配置」一并保存，避免用户以为还要
      // 单独点一个 Key 保存按钮（该按钮已移除）。
      const pendingKey = apiKey.trim();
      if (pendingKey) {
        const savedKey = await safeInvoke("save_api_key", {
          provider: providerAtStart,
          key: pendingKey,
        });
        if (!savedKey.ok) {
          setStatus(`保存 API Key 失败：${savedKey.error}`);
          return false;
        }
        // 期间切了协议：输入框里现在可能是新协议刚粘的 Key，不能清。
        if (providerRef.current === providerAtStart) setApiKey("");
      }
      const settings: Settings = {
        ...form,
        baseUrl: normalizeBaseUrl(form.baseUrl, form.provider) || form.baseUrl,
      };
      if (settings.baseUrl !== form.baseUrl) patch({ baseUrl: settings.baseUrl });
      const result = await safeInvoke("save_settings", { settings });
      if (!result.ok) {
        // 后端 save_settings 的所有错误都以「保存配置失败：」开头，
        // 这里再拼一次前缀会出现「保存失败：保存配置失败：…」。
        setStatus(result.error);
        return false;
      }
      if (providerRef.current !== providerAtStart) {
        setStatus(
          `已按「${providerLabel(providerAtStart)}」保存配置；期间切换了协议，当前协议尚未保存，请再点一次「保存全部配置」。`,
        );
        return true;
      }
      const present = await hasKeyFor(providerAtStart);
      setStatus(
        present
          ? "配置已保存"
          : "配置已保存，但当前接口还没有保存 API Key，开始面试会失败。请在上方填入 Key 并点「保存全部配置」。",
      );
      return true;
    } finally {
      setBusy(false);
      setAction("");
    }
  }

  async function loadModels(): Promise<void> {
    setBusy(true);
    setAction("models");
    setStatus("");
    const requestId = ++modelsRequest.current;
    try {
      // 用当前表单（含未保存的 baseUrl / Key）直接拉取，不需要先保存。
      const pendingKey = apiKey.trim();
      const result = await safeInvoke<unknown>("fetch_models", {
        settings: {
          ...form,
          baseUrl: normalizeBaseUrl(form.baseUrl, form.provider) || form.baseUrl,
        },
        apiKey: pendingKey,
      });
      // 请求期间切换了协议/Base URL：这批模型不属于当前配置，直接丢弃。
      if (modelsRequest.current !== requestId) return;
      if (!result.ok) {
        // 后端错误已带「拉取模型列表失败：」前缀，避免重复拼接。
        const prefix = "拉取模型列表失败：";
        const detail = result.error.startsWith(prefix)
          ? result.error.slice(prefix.length)
          : result.error;
        setStatus(`拉取模型列表失败：${detail}`);
        return;
      }
      // 后端保证返回字符串数组，但前端仍做类型防御：异常响应不应让设置页报错。
      const raw = Array.isArray(result.data) ? result.data : [];
      const list = Array.from(
        new Set(
          raw
            .filter((item): item is string => typeof item === "string")
            .map((item) => item.trim())
            .filter((item) => item !== ""),
        ),
      );
      setModels(list);
      setStatus(
        list.length
          ? `已获取 ${list.length} 个可用模型，可在模型名处下拉选择`
          : "接口没有返回可用模型，请手动填写模型名",
      );
    } finally {
      setBusy(false);
      setAction("");
    }
  }

  async function testConnection(): Promise<{ text: string; model: string } | null> {
    setBusy(true);
    setAction("test");
    setStatus("");
    const providerAtStart = form.provider;
    try {
      // 把当前表单一起发过去：改了 baseUrl/model 不保存也能直接测。
      // 表单里刚输入、尚未保存的 Key 也直接带上，不需要先点「保存」。
      const pendingKey = apiKey.trim();
      const result = await safeInvoke<unknown>("test_llm", {
        prompt: "请用一句话介绍你自己，并确认你能正常工作。",
        settings: {
          ...form,
          baseUrl: normalizeBaseUrl(form.baseUrl, form.provider) || form.baseUrl,
        },
        apiKey: pendingKey,
      });
      if (!result.ok) {
        // 后端返回的已是「模型连接失败：…」，去掉外层前缀避免重复。
        const prefix = "模型连接失败：";
        const detail = result.error.startsWith(prefix)
          ? result.error.slice(prefix.length)
          : result.error;
        setStatus(`连接失败：${detail}`);
        return null;
      }
      const data = result.data as { text?: string; model?: string } | null;
      const text = typeof data?.text === "string" ? data.text : "";
      const model = typeof data?.model === "string" ? data.model : form.model;
      // 测试期间切了协议：这份结果属于旧协议，不能把 Key 状态写到新协议上，
      // 也不能清掉用户为新协议刚粘的 Key。
      if (providerRef.current !== providerAtStart) {
        setStatus(
          `连接成功，模型：${model}（测试用的是「${providerLabel(providerAtStart)}」，期间已切换协议，未自动保存 Key）`,
        );
        return { text, model };
      }
      // 测试成功且表单里有 Key：顺手保存，减少「测通了但没存」的第二次操作。
      if (pendingKey) {
        const savedKey = await safeInvoke("save_api_key", {
          provider: providerAtStart,
          key: pendingKey,
        });
        if (savedKey.ok) {
          setApiKey("");
          await hasKeyFor(providerAtStart);
          setStatus(`连接成功，模型：${model}；API Key 已保存`);
          return { text, model };
        }
        setStatus(`连接成功，模型：${model}；但保存 API Key 失败：${savedKey.error}`);
        return { text, model };
      }
      setStatus(`连接成功，模型：${model}`);
      return { text, model };
    } finally {
      setBusy(false);
      setAction("");
    }
  }

  async function loadDevices() {
    const result = await safeInvoke<unknown>("list_audio_devices");
    if (!result.ok) {
      return { inputs: [], outputs: [], error: result.error };
    }
    return {
      inputs: normalizeDevices(result.data, "inputs"),
      outputs: normalizeDevices(result.data, "outputs"),
      error: "",
    };
  }

  return {
    form,
    apiKey,
    setApiKey,
    keySaved,
    loaded,
    status,
    setStatus,
    busy,
    action,
    patch,
    save,
    testConnection,
    loadModels,
    models,
    loadDevices,
  };
}

/** 协议中文短名；只在提示文案里用，与设置页下拉的写法保持一致。 */
function providerLabel(provider: Settings["provider"]): string {
  switch (provider) {
    case "anthropic":
      return "Anthropic Messages";
    case "gemini":
      return "Gemini generateContent";
    default:
      return "OpenAI 兼容";
  }
}
