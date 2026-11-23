import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

type BackendCall = { command: string; args: unknown };

const backend = vi.hoisted(() => {
  const calls: BackendCall[] = [];
  const routes = new Map<string, (args: unknown) => unknown>();
  return { calls, routes };
});

// safeInvoke 只关心 @tauri-apps/api/core 的 invoke：拒绝（抛错）代表后端失败，
// 返回原始数据代表成功，由 safeInvoke 包成 { ok, data | error }。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    const handler = backend.routes.get(command);
    return Promise.resolve().then(() => (handler ? handler(args) : null));
  }),
}));

import { useSettingsForm } from "./useSettingsForm";
import { DEFAULT_OVERLAY_OPACITY, EMPTY_SETTINGS } from "./types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function route<T>(command: string, handler: (args: unknown) => T) {
  backend.routes.set(command, handler as (args: unknown) => unknown);
}

function callsTo(command: string): BackendCall[] {
  return backend.calls.filter((call) => call.command === command);
}

function onlyArgs(command: string): unknown[] {
  return callsTo(command).map((call) => call.args);
}

/** 让所有在途的微任务与 effect 落定，再断言。 */
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function mount() {
  const utils = renderHook(() => useSettingsForm());
  await waitFor(() => {
    expect(utils.result.current.loaded).toBe(true);
  });
  await flush();
  return utils;
}

beforeEach(() => {
  backend.calls.length = 0;
  backend.routes.clear();
  route("get_settings", () => ({
    provider: "openai",
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
  }));
  route("has_api_key", () => false);
});

describe("useSettingsForm —— 读取配置", () => {
  it("读取成功：配置归一化后进表单，loaded=true，无提示", async () => {
    route("get_settings", () => ({
      provider: "anthropic",
      baseUrl: "https://api.anthropic.com",
      model: "claude-sonnet-5-5",
      resume: "简历正文",
      projects: "项目正文",
      answerStyle: "outline",
      triggerMode: "manual",
      overlayOpacity: 0.5,
    }));

    const { result } = await mount();
    expect(result.current.form.provider).toBe("anthropic");
    expect(result.current.form.model).toBe("claude-sonnet-5-5");
    expect(result.current.form.resume).toBe("简历正文");
    expect(result.current.form.answerStyle).toBe("outline");
    expect(result.current.form.triggerMode).toBe("manual");
    expect(result.current.form.overlayOpacity).toBe(0.5);
    expect(result.current.status).toBe("");
    expect(result.current.loaded).toBe(true);
    expect(result.current.busy).toBe(false);
    expect(result.current.action).toBe("");
  });

  it("读取成功但字段残缺：缺的设备字段补空、不透明度回落默认值", async () => {
    route("get_settings", () => ({ provider: "gemini", model: "gemini-2.5-flash" }));
    const { result } = await mount();
    expect(result.current.form.provider).toBe("gemini");
    expect(result.current.form.baseUrl).toBe("");
    expect(result.current.form.micDevice).toBe("");
    expect(result.current.form.outputDevice).toBe("");
    expect(result.current.form.overlayOpacity).toBe(DEFAULT_OVERLAY_OPACITY);
  });

  it("读取失败：状态里带中文提示，表单退回默认值且仍然放行渲染", async () => {
    route("get_settings", () => Promise.reject("配置文件损坏"));
    const { result } = await mount();
    expect(result.current.status).toBe("读取配置失败：配置文件损坏。已使用默认值。");
    expect(result.current.form).toEqual(EMPTY_SETTINGS);
    expect(result.current.loaded).toBe(true);
  });

  it("非 Tauri 环境（删掉 __TAURI_INTERNALS__）走降级文案，不抛异常", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    try {
      const { result } = await mount();
      expect(result.current.status).toBe(
        "读取配置失败：当前不在 Tauri 环境中运行，后端命令不可用。已使用默认值。",
      );
      expect(result.current.form).toEqual(EMPTY_SETTINGS);
    } finally {
      if (descriptor) Object.defineProperty(window, "__TAURI_INTERNALS__", descriptor);
    }
  });

  it("读取期间卸载：迟到的响应不写状态", async () => {
    const pending = deferred<unknown>();
    route("get_settings", () => pending.promise);
    const { result, unmount } = renderHook(() => useSettingsForm());
    expect(result.current.loaded).toBe(false);
    unmount();
    await act(async () => {
      pending.resolve({ provider: "gemini", model: "late" });
    });
    expect(result.current.loaded).toBe(false);
    expect(result.current.form.model).toBe("");
  });
});

describe("useSettingsForm —— Key 状态跟随协议", () => {
  it("挂载时按当前协议查 Key；切协议后重新查询并清空输入框", async () => {
    route("has_api_key", (args) =>
      (args as { provider: string }).provider === "gemini" ? true : false,
    );
    const { result } = await mount();
    expect(onlyArgs("has_api_key")).toEqual([{ provider: "openai" }]);
    expect(result.current.keySaved).toBe(false);

    act(() => {
      result.current.setApiKey("sk-旧协议");
    });
    expect(result.current.apiKey).toBe("sk-旧协议");

    act(() => {
      result.current.patch({ provider: "gemini" });
    });
    await flush();
    expect(onlyArgs("has_api_key")).toContainEqual({ provider: "gemini" });
    expect(result.current.keySaved).toBe(true);
    // 切协议时输入框必须清空：A 的 Key 不能留到 B 上
    expect(result.current.apiKey).toBe("");
  });

  it("切协议或改 Base URL 都会丢弃上一批模型列表", async () => {
    route("fetch_models", () => ["model-a"]);
    const { result } = await mount();
    await act(async () => {
      await result.current.loadModels();
    });
    expect(result.current.models).toEqual(["model-a"]);

    act(() => {
      result.current.patch({ provider: "anthropic" });
    });
    await flush();
    expect(result.current.models).toEqual([]);

    route("fetch_models", () => ["model-b"]);
    await act(async () => {
      await result.current.loadModels();
    });
    expect(result.current.models).toEqual(["model-b"]);

    act(() => {
      result.current.patch({ baseUrl: "https://api.anthropic.com/v2" });
    });
    await flush();
    expect(result.current.models).toEqual([]);
  });
});

describe("useSettingsForm —— patch", () => {
  it("局部合并：只覆盖传入字段，其余保持", async () => {
    const { result } = await mount();
    const before = result.current.form;
    act(() => {
      result.current.patch({ model: "deepseek-reasoner", overlayOpacity: 0.6 });
    });
    expect(result.current.form.model).toBe("deepseek-reasoner");
    expect(result.current.form.overlayOpacity).toBe(0.6);
    expect(result.current.form.baseUrl).toBe(before.baseUrl);
    expect(result.current.form.provider).toBe(before.provider);
    expect(result.current.form.answerStyle).toBe(before.answerStyle);

    act(() => {
      result.current.patch({ resume: "只改简历" });
    });
    expect(result.current.form.resume).toBe("只改简历");
    expect(result.current.form.model).toBe("deepseek-reasoner");
  });
});

describe("useSettingsForm —— save", () => {
  it("表单里有 Key：先 save_api_key，再 save_settings，成功后清空输入框并刷新 Key 状态", async () => {
    route("save_api_key", () => null);
    route("save_settings", () => null);
    route("has_api_key", () => true);
    const { result } = await mount();

    act(() => {
      result.current.setApiKey("  sk-abc  ");
    });
    let saved: boolean | undefined;
    await act(async () => {
      saved = await result.current.save();
    });

    expect(saved).toBe(true);
    expect(onlyArgs("save_api_key")).toEqual([{ provider: "openai", key: "sk-abc" }]);
    expect(callsTo("save_settings")).toHaveLength(1);
    expect(result.current.apiKey).toBe("");
    expect(result.current.keySaved).toBe(true);
    expect(result.current.status).toBe("配置已保存");
    expect(result.current.busy).toBe(false);
    expect(result.current.action).toBe("");
  });

  it("没有 Key：不发 save_api_key，提示「还没有保存 API Key」", async () => {
    route("save_settings", () => null);
    route("has_api_key", () => false);
    const { result } = await mount();

    await act(async () => {
      await result.current.save();
    });

    expect(callsTo("save_api_key")).toHaveLength(0);
    expect(callsTo("save_settings")).toHaveLength(1);
    expect(result.current.status).toContain("配置已保存，但当前接口还没有保存 API Key");
    expect(result.current.keySaved).toBe(false);
  });

  it("save_api_key 失败：带前缀报错，且不写配置", async () => {
    route("save_api_key", () => Promise.reject("钥匙串不可用"));
    const { result } = await mount();
    act(() => {
      result.current.setApiKey("sk-x");
    });

    let saved: boolean | undefined;
    await act(async () => {
      saved = await result.current.save();
    });

    expect(saved).toBe(false);
    expect(result.current.status).toBe("保存 API Key 失败：钥匙串不可用");
    expect(callsTo("save_settings")).toHaveLength(0);
    // 失败后输入框保留用户的 Key，方便重试
    expect(result.current.apiKey).toBe("sk-x");
    expect(result.current.busy).toBe(false);
    expect(result.current.action).toBe("");
  });

  it("save_settings 失败：提示「保存失败：…」并返回 false", async () => {
    route("save_settings", () => Promise.reject("磁盘只读"));
    const { result } = await mount();

    let saved: boolean | undefined;
    await act(async () => {
      saved = await result.current.save();
    });
    expect(saved).toBe(false);
    expect(result.current.status).toBe("保存失败：磁盘只读");
    expect(result.current.busy).toBe(false);
  });

  it("保存中 busy=true / action=save，命令返回后复位", async () => {
    const pending = deferred<unknown>();
    route("save_settings", () => pending.promise);
    const { result } = await mount();

    let running!: Promise<boolean>;
    act(() => {
      running = result.current.save();
    });
    await flush();
    expect(result.current.busy).toBe(true);
    expect(result.current.action).toBe("save");

    await act(async () => {
      pending.resolve(null);
      await running;
    });
    expect(result.current.busy).toBe(false);
    expect(result.current.action).toBe("");
  });

  it("保存时规范化 Base URL：补协议头 + /v1，并把结果写回表单", async () => {
    route("save_settings", () => null);
    const { result } = await mount();
    act(() => {
      result.current.patch({ baseUrl: "api.deepseek.com" });
    });

    await act(async () => {
      await result.current.save();
    });

    const args = onlyArgs("save_settings")[0] as { settings: { baseUrl: string } };
    expect(args.settings.baseUrl).toBe("https://api.deepseek.com/v1");
    expect(result.current.form.baseUrl).toBe("https://api.deepseek.com/v1");
  });

  it("保存期间切换协议：不清空新协议刚粘的 Key，并提示当前协议未保存（providerRef 守卫）", async () => {
    const saveKeyPending = deferred<unknown>();
    route("save_api_key", () => saveKeyPending.promise);
    route("save_settings", () => null);
    const { result } = await mount();

    act(() => {
      result.current.setApiKey("sk-openai");
    });
    let running!: Promise<boolean>;
    act(() => {
      running = result.current.save();
    });
    await flush();
    expect(result.current.action).toBe("save");

    // 后端还在保存 Key，表单已切到 Anthropic，用户又粘了 Anthropic 的 Key
    act(() => {
      result.current.patch({ provider: "anthropic" });
    });
    await flush();
    expect(result.current.apiKey).toBe("");
    act(() => {
      result.current.setApiKey("sk-anthropic");
    });

    await act(async () => {
      saveKeyPending.resolve(null);
      await running;
    });

    // 守卫：结果写回时协议已经不是发起时的那个，不能清掉新协议的输入
    expect(result.current.apiKey).toBe("sk-anthropic");
    expect(result.current.status).toContain("已按「OpenAI 兼容」保存配置");
    expect(result.current.status).toContain("期间切换了协议");
    // 保存的 Key 归属发起时的协议
    expect(onlyArgs("save_api_key")).toEqual([
      { provider: "openai", key: "sk-openai" },
    ]);
  });
it("保存期间切到 Anthropic / Gemini：提示里的协议名用中文短名", async () => {
    const pending = deferred<unknown>();
    route("save_settings", () => pending.promise);
    const { result } = await mount();

    act(() => {
      result.current.patch({ provider: "gemini" });
    });
    await flush();
    act(() => {
      result.current.patch({ provider: "anthropic" });
    });
    await flush();

    // 先发起保存（以 anthropic 为起点），期间再切到 gemini
    let running!: Promise<boolean>;
    act(() => {
      running = result.current.save();
    });
    await flush();
    act(() => {
      result.current.patch({ provider: "gemini" });
    });

    await act(async () => {
      pending.resolve(null);
      await running;
    });
    expect(result.current.status).toContain("Anthropic Messages");
  });

  it("以 Gemini 为起点保存期间切协议：提示里出现「Gemini generateContent」", async () => {
    const pending = deferred<unknown>();
    route("save_settings", () => pending.promise);
    const { result } = await mount();
    act(() => {
      result.current.patch({ provider: "gemini" });
    });
    await flush();

    let running!: Promise<boolean>;
    act(() => {
      running = result.current.save();
    });
    await flush();
    act(() => {
      result.current.patch({ provider: "openai" });
    });

    await act(async () => {
      pending.resolve(null);
      await running;
    });
    expect(result.current.status).toContain("已按「Gemini generateContent」保存配置");
  });
});

describe("useSettingsForm —— testConnection", () => {
  it("用表单里的 Key 测试成功：顺手保存 Key 并回传文本与模型", async () => {
    route("test_llm", () => ({ text: "我是模型", model: "deepseek-chat" }));
    route("save_api_key", () => null);
    route("has_api_key", () => true);
    const { result } = await mount();
    act(() => {
      result.current.setApiKey("sk-test");
    });

    let response: { text: string; model: string } | null = null;
    await act(async () => {
      response = await result.current.testConnection();
    });

    expect(response).toEqual({ text: "我是模型", model: "deepseek-chat" });
    const llmArgs = onlyArgs("test_llm")[0] as { apiKey: string; prompt: string };
    expect(llmArgs.apiKey).toBe("sk-test");
    expect(llmArgs.prompt).toContain("请用一句话介绍你自己");
    expect(onlyArgs("save_api_key")).toEqual([
      { provider: "openai", key: "sk-test" },
    ]);
    expect(result.current.apiKey).toBe("");
    expect(result.current.keySaved).toBe(true);
    expect(result.current.status).toBe("连接成功，模型：deepseek-chat；API Key 已保存");
  });

  it("用已保存的 Key 测试：不带 apiKey、不重复保存", async () => {
    route("test_llm", () => ({ text: "ok", model: "gpt-4o-mini" }));
    const { result } = await mount();

    let response: { text: string; model: string } | null = null;
    await act(async () => {
      response = await result.current.testConnection();
    });

    expect(response).toEqual({ text: "ok", model: "gpt-4o-mini" });
    const llmArgs = onlyArgs("test_llm")[0] as { apiKey: string };
    expect(llmArgs.apiKey).toBe("");
    expect(callsTo("save_api_key")).toHaveLength(0);
    expect(result.current.status).toBe("连接成功，模型：gpt-4o-mini");
  });

  it("测试失败：剥掉后端前缀，返回 null", async () => {
    route("test_llm", () => Promise.reject("模型连接失败：认证失败"));
    const { result } = await mount();

    let response: { text: string; model: string } | null = { text: "", model: "" };
    await act(async () => {
      response = await result.current.testConnection();
    });

    expect(response).toBeNull();
    expect(result.current.status).toBe("连接失败：认证失败");
    expect(result.current.busy).toBe(false);
    expect(result.current.action).toBe("");
  });

  it("测试成功但 save_api_key 失败：仍回传结果，提示 Key 未保存", async () => {
    route("test_llm", () => ({ text: "ok", model: "m1" }));
    route("save_api_key", () => Promise.reject("无权限"));
    const { result } = await mount();
    act(() => {
      result.current.setApiKey("sk-1");
    });

    let response: { text: string; model: string } | null = null;
    await act(async () => {
      response = await result.current.testConnection();
    });

    expect(response).toEqual({ text: "ok", model: "m1" });
    expect(result.current.status).toBe("连接成功，模型：m1；但保存 API Key 失败：无权限");
    expect(result.current.apiKey).toBe("sk-1");
  });

  it("测试期间切换协议：不保存 Key、不清输入框，提示未自动保存", async () => {
    const pending = deferred<unknown>();
    route("test_llm", () => pending.promise);
    const { result } = await mount();
    act(() => {
      result.current.setApiKey("sk-openai");
    });

    let running!: Promise<{ text: string; model: string } | null>;
    act(() => {
      running = result.current.testConnection();
    });
    await flush();

    act(() => {
      result.current.patch({ provider: "anthropic" });
    });
    await flush();
    act(() => {
      result.current.setApiKey("sk-anthropic");
    });

    await act(async () => {
      pending.resolve({ text: "ok", model: "claude-sonnet-5-5" });
      await running;
    });

    expect(callsTo("save_api_key")).toHaveLength(0);
    expect(result.current.apiKey).toBe("sk-anthropic");
    expect(result.current.status).toContain("期间已切换协议，未自动保存 Key");
    // 文案里的协议名是「发起测试时」的那个，不是切换后的
    expect(result.current.status).toContain("OpenAI 兼容");
  });

  it("返回值缺字段时 model 回落表单值、text 回落空串", async () => {
    route("test_llm", () => ({ unexpected: true }));
    const { result } = await mount();

    let response: { text: string; model: string } | null = null;
    await act(async () => {
      response = await result.current.testConnection();
    });
    expect(response).toEqual({ text: "", model: "deepseek-chat" });
  });
});

describe("useSettingsForm —— loadModels", () => {
  it("成功：去重、trim、过滤非字符串与空值，并给出可下拉选择的提示", async () => {
    route("fetch_models", () => ["a", " a ", "b", "", 42, null, "  "]);
    const { result } = await mount();

    await act(async () => {
      await result.current.loadModels();
    });

    expect(result.current.models).toEqual(["a", "b"]);
    expect(result.current.status).toBe("已获取 2 个可用模型，可在模型名处下拉选择");
    expect(result.current.busy).toBe(false);
    expect(result.current.action).toBe("");
  });

  it("接口没返回模型：提示手动填写", async () => {
    route("fetch_models", () => []);
    const { result } = await mount();

    await act(async () => {
      await result.current.loadModels();
    });
    expect(result.current.models).toEqual([]);
    expect(result.current.status).toBe("接口没有返回可用模型，请手动填写模型名");
  });

  it("返回非数组也不崩：按空列表处理", async () => {
    route("fetch_models", () => ({ models: ["x"] }));
    const { result } = await mount();

    await act(async () => {
      await result.current.loadModels();
    });
    expect(result.current.models).toEqual([]);
    expect(result.current.status).toBe("接口没有返回可用模型，请手动填写模型名");
  });

  it("失败：后端已带前缀时不重复拼接", async () => {
    route("fetch_models", () => Promise.reject("拉取模型列表失败：401 未授权"));
    const { result } = await mount();

    await act(async () => {
      await result.current.loadModels();
    });
    expect(result.current.status).toBe("拉取模型列表失败：401 未授权");
    expect(result.current.busy).toBe(false);
  });

  it("失败：后端不带前缀时补上", async () => {
    route("fetch_models", () => Promise.reject("网络不可达"));
    const { result } = await mount();

    await act(async () => {
      await result.current.loadModels();
    });
    expect(result.current.status).toBe("拉取模型列表失败：网络不可达");
  });

  it("旧请求的响应晚到：不覆盖新请求的结果（序号守卫）", async () => {
    const pendings: Array<ReturnType<typeof deferred<unknown>>> = [];
    route("fetch_models", () => {
      const pending = deferred<unknown>();
      pendings.push(pending);
      return pending.promise;
    });
    const { result } = await mount();

    await act(async () => {
      void result.current.loadModels();
      void result.current.loadModels();
    });
    expect(pendings).toHaveLength(2);

    await act(async () => {
      pendings[1].resolve(["新模型"]);
    });
    await flush();
    expect(result.current.models).toEqual(["新模型"]);

    // 第一次请求（已过期）此刻才返回，不得覆盖
    await act(async () => {
      pendings[0].resolve(["旧模型"]);
    });
    await flush();
    expect(result.current.models).toEqual(["新模型"]);
  });

  it("拉取期间切协议：响应被丢弃", async () => {
    const pending = deferred<unknown>();
    route("fetch_models", () => pending.promise);
    const { result } = await mount();

    await act(async () => {
      void result.current.loadModels();
    });
    act(() => {
      result.current.patch({ provider: "gemini" });
    });
    await flush();

    await act(async () => {
      pending.resolve(["不该出现的模型"]);
    });
    await flush();
    expect(result.current.models).toEqual([]);
  });

  it("拉取时把未保存的 Base URL 规范化后一起发出", async () => {
    route("fetch_models", () => ["m"]);
    const { result } = await mount();
    act(() => {
      result.current.patch({ baseUrl: "api.openai.com/chat/completions" });
    });
    act(() => {
      result.current.setApiKey("sk-pending");
    });

    await act(async () => {
      await result.current.loadModels();
    });

    const args = onlyArgs("fetch_models")[0] as {
      apiKey: string;
      settings: { baseUrl: string };
    };
    expect(args.apiKey).toBe("sk-pending");
    expect(args.settings.baseUrl).toBe("https://api.openai.com/v1");
  });
});

describe("useSettingsForm —— Base URL 规范化触发点", () => {
  it("Base URL 为空：save / loadModels / testConnection 都不补默认值，原样透传空串", async () => {
    route("save_settings", () => null);
    route("fetch_models", () => ["m"]);
    route("test_llm", () => ({ text: "ok", model: "m" }));
    const { result } = await mount();
    act(() => {
      result.current.patch({ baseUrl: "" });
    });

    await act(async () => {
      await result.current.save();
    });
    await act(async () => {
      await result.current.loadModels();
    });
    await act(async () => {
      await result.current.testConnection();
    });

    const saved = onlyArgs("save_settings")[0] as { settings: { baseUrl: string } };
    const fetched = onlyArgs("fetch_models")[0] as { settings: { baseUrl: string } };
    const tested = onlyArgs("test_llm")[0] as { settings: { baseUrl: string } };
    expect(saved.settings.baseUrl).toBe("");
    expect(fetched.settings.baseUrl).toBe("");
    expect(tested.settings.baseUrl).toBe("");
    // 空串不需要回写，表单保持用户输入
    expect(result.current.form.baseUrl).toBe("");
  });

  it("Base URL 已规范（带 /v1）时不再改动表单，也不多加路径", async () => {
    route("save_settings", () => null);
    const { result } = await mount();
    expect(result.current.form.baseUrl).toBe("https://api.deepseek.com/v1");

    await act(async () => {
      await result.current.save();
    });
    const saved = onlyArgs("save_settings")[0] as { settings: { baseUrl: string } };
    expect(saved.settings.baseUrl).toBe("https://api.deepseek.com/v1");
    expect(result.current.form.baseUrl).toBe("https://api.deepseek.com/v1");
  });

  it("测试失败但后端文案不带「模型连接失败：」前缀时原样展示", async () => {
    route("test_llm", () => Promise.reject("网络不可达"));
    const { result } = await mount();

    await act(async () => {
      await result.current.testConnection();
    });
    expect(result.current.status).toBe("连接失败：网络不可达");
  });
});

describe("useSettingsForm —— loadDevices", () => {
  it("成功：inputs / outputs 分别归一化", async () => {
    route("list_audio_devices", () => ({
      inputs: [{ id: "mic-1", name: "麦克风" }, "mic-2"],
      outputs: [{ deviceId: "spk-1", label: "扬声器" }],
    }));
    const { result } = await mount();

    let devices = { inputs: [] as unknown[], outputs: [] as unknown[], error: "" };
    await act(async () => {
      devices = await result.current.loadDevices();
    });
    expect(devices.inputs).toEqual([
      { id: "mic-1", name: "麦克风" },
      { id: "mic-2", name: "mic-2" },
    ]);
    expect(devices.outputs).toEqual([{ id: "spk-1", name: "扬声器" }]);
    expect(devices.error).toBe("");
  });

  it("失败：返回空列表与错误文案", async () => {
    route("list_audio_devices", () => Promise.reject("音频服务未启动"));
    const { result } = await mount();

    let devices = { inputs: [] as unknown[], outputs: [] as unknown[], error: "" };
    await act(async () => {
      devices = await result.current.loadDevices();
    });
    expect(devices.inputs).toEqual([]);
    expect(devices.outputs).toEqual([]);
    expect(devices.error).toBe("音频服务未启动");
  });
});