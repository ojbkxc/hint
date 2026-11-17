import { useCallback, useEffect, useState } from "react";
import { safeInvoke } from "../lib/tauri";
import {
  EMPTY_SETTINGS,
  normalizeDevices,
  normalizeSettings,
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

  useEffect(() => {
    let disposed = false;
    void (async () => {
      const settings = await safeInvoke<unknown>("get_settings");
      if (disposed) return;
      if (settings.ok) {
        const normalized = normalizeSettings(settings.data);
        setForm(normalized);
        const saved = await safeInvoke<unknown>("has_api_key", {
          provider: normalized.provider,
        });
        if (!disposed && saved.ok) setKeySaved(saved.data === true);
      } else {
        setStatus(`读取配置失败：${settings.error}。已使用默认值。`);
      }
      if (!disposed) setLoaded(true);
    })();
    return () => {
      disposed = true;
    };
  }, []);

  const patch = useCallback((next: Partial<Settings>) => {
    setForm((current) => ({ ...current, ...next }));
  }, []);

  async function save() {
    setBusy(true);
    setStatus("");
    try {
      const result = await safeInvoke("save_settings", { settings: form });
      setStatus(result.ok ? "配置已保存" : `保存失败：${result.error}`);
      return result.ok;
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
      const result = await safeInvoke("save_api_key", {
        provider: form.provider,
        key: apiKey.trim(),
      });
      if (result.ok) {
        setKeySaved(true);
        setApiKey("");
        setStatus("API Key 已写入 Windows 凭据管理器");
      } else {
        setStatus(`保存 Key 失败：${result.error}`);
      }
    } finally {
      setBusy(false);
    }
  }

  async function testConnection(): Promise<{ text: string; model: string } | null> {
    setBusy(true);
    setStatus("");
    try {
      // 把当前表单一起发过去：改了 baseUrl/model 不保存也能直接测。
      const result = await safeInvoke<unknown>("test_llm", {
        prompt: "请用一句话介绍你自己，并确认你能正常工作。",
        settings: form,
      });
      if (!result.ok) {
        setStatus(`连接失败：${result.error}`);
        return null;
      }
      const data = result.data as { text?: string; model?: string } | null;
      const text = typeof data?.text === "string" ? data.text : "";
      const model = typeof data?.model === "string" ? data.model : form.model;
      setStatus(`连接成功，模型：${model}`);
      return { text, model };
    } finally {
      setBusy(false);
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
    patch,
    save,
    saveKey,
    testConnection,
    loadDevices,
  };
}