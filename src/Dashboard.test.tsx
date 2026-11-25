import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type BackendCall = { command: string; args: unknown };

const backend = vi.hoisted(() => {
  const calls: BackendCall[] = [];
  const routes = new Map<string, (args: unknown) => unknown>();
  return { calls, routes };
});

const bus = vi.hoisted(() => {
  type Handler = (payload: unknown) => void;
  const handlers = new Map<string, Handler[]>();
  return {
    handlers,
    emit(event: string, payload: unknown) {
      for (const handler of handlers.get(event) ?? []) handler(payload);
    },
  };
});

/**
 * Dashboard 聚合四个 hook。这里用可替换的对象持有它们的返回值：
 * 测试在渲染前后改字段即可改变派生结果，不需要真的挂一遍后端状态机。
 * 组件本身（ModelCard / ModelDownloadCard / … / SessionReview）保持真实，
 * 这样断言打的是 Dashboard 的装配与派生逻辑，而不是假组件。
 */
const hooks = vi.hoisted(() => ({
  settings: null as unknown,
  model: null as unknown,
  session: null as unknown,
  sessions: null as unknown,
  /** useSessions 的 enabled 实参历史；用来确认只有复盘 Tab 才拉列表。 */
  sessionsEnabled: [] as boolean[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    const handler = backend.routes.get(command);
    return Promise.resolve().then(() => (handler ? handler(args) : null));
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (incoming: { payload: unknown }) => void) => {
    const wrapped = (payload: unknown) => handler({ payload });
    const list = bus.handlers.get(event) ?? [];
    list.push(wrapped);
    bus.handlers.set(event, list);
    return Promise.resolve(() => {
      bus.handlers.set(
        event,
        (bus.handlers.get(event) ?? []).filter((item) => item !== wrapped),
      );
    });
  }),
}));

vi.mock("./settings/useSettingsForm", () => ({
  useSettingsForm: () => hooks.settings,
}));
vi.mock("./settings/useModelManager", () => ({
  useModelManager: () => hooks.model,
}));
vi.mock("./settings/useSessionControl", () => ({
  useSessionControl: () => hooks.session,
}));
vi.mock("./sessions/useSessions", () => ({
  useSessions: (enabled: boolean) => {
    hooks.sessionsEnabled.push(enabled);
    return hooks.sessions;
  },
}));

import Dashboard from "./Dashboard";
import { INITIAL_MODEL_VIEW, type ModelView } from "./settings/modelTypes";
import { EMPTY_SETTINGS, type Settings } from "./settings/types";

function makeForm(overrides: Record<string, unknown> = {}) {
  return {
    form: {
      ...EMPTY_SETTINGS,
      baseUrl: "https://api.deepseek.com/v1",
      model: "deepseek-chat",
    } as Settings,
    apiKey: "",
    setApiKey: vi.fn(),
    keySaved: true,
    loaded: true,
    status: "",
    setStatus: vi.fn(),
    busy: false,
    action: "",
    patch: vi.fn(),
    save: vi.fn(() => Promise.resolve(true)),
    testConnection: vi.fn(() => Promise.resolve(null)),
    loadModels: vi.fn(() => Promise.resolve()),
    models: [] as string[],
    loadDevices: vi.fn(() =>
      Promise.resolve({ inputs: [], outputs: [], error: "" }),
    ),
    ...overrides,
  };
}

function makeModel(overrides: Record<string, unknown> = {}) {
  return {
    view: { ...INITIAL_MODEL_VIEW, phase: "ready", totalBytes: 241_357_257 } as ModelView,
    busy: false,
    notice: "",
    download: vi.fn(() => Promise.resolve()),
    cancel: vi.fn(() => Promise.resolve()),
    remove: vi.fn(() => Promise.resolve()),
    refresh: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

function makeSession(overrides: Record<string, unknown> = {}) {
  return {
    state: "idle",
    message: "",
    running: false,
    busy: false,
    busyAction: "",
    error: "",
    notice: "",
    sessionId: null,
    start: vi.fn(),
    stop: vi.fn(),
    ...overrides,
  };
}

function makeSessions(overrides: Record<string, unknown> = {}) {
  return {
    sessions: [],
    loading: false,
    error: "",
    selectedId: "",
    detail: null,
    detailLoading: false,
    detailError: "",
    notice: "",
    refresh: vi.fn(() => Promise.resolve()),
    open: vi.fn(() => Promise.resolve()),
    exportMarkdown: vi.fn(() => Promise.resolve()),
    remove: vi.fn(() => Promise.resolve()),
    removeAll: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

/** 等 DevicesCard 首次拉设备的 Promise 落定，避免 act 告警。 */
async function mount() {
  let utils!: ReturnType<typeof render>;
  await act(async () => {
    utils = render(<Dashboard />);
  });
  return utils;
}

function emit(event: string, payload: unknown) {
  act(() => {
    bus.emit(event, payload);
  });
}

function callsTo(command: string): BackendCall[] {
  return backend.calls.filter((call) => call.command === command);
}

function railItem(name: string): HTMLElement {
  return screen.getByRole("button", { name: new RegExp(name) });
}

function railBadge(name: string): HTMLElement | null {
  return railItem(name).querySelector(".settings-rail-badge");
}

function sectionOf(container: HTMLElement, id: string): HTMLElement {
  const node = container.querySelector(`#section-${id}`) as HTMLElement | null;
  expect(node).not.toBeNull();
  return node as HTMLElement;
}

/** 七个 section 的可见集合：只有 hidden=false 的那一个算可见。 */
function visibleSections(container: HTMLElement): string[] {
  const ids = ["model", "voice", "session", "devices", "profile", "behavior", "save"];
  return ids.filter((id) => !sectionOf(container, id).hidden);
}

beforeEach(() => {
  backend.calls.length = 0;
  backend.routes.clear();
  bus.handlers.clear();
  hooks.sessionsEnabled.length = 0;
  hooks.settings = makeForm();
  hooks.model = makeModel();
  hooks.session = makeSession();
  hooks.sessions = makeSessions();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Dashboard —— 骨架态", () => {
  it("未 loaded 时 shell 标 aria-busy，rail 与正文各渲染骨架", async () => {
    hooks.settings = makeForm({ loaded: false });
    const { container } = await mount();

    const shell = container.querySelector(".settings-shell") as HTMLElement;
    expect(shell).toHaveAttribute("aria-busy", "true");
    expect(screen.getAllByText("正在读取配置").length).toBe(2); // 骨架标题 + 屏读文案
    // 骨架 rail 7 条（与真实 rail 条目数一致），正文 3 块占位
    expect(
      container.querySelectorAll(".settings-rail-skeleton .skeleton").length,
    ).toBe(7);
    expect(container.querySelectorAll(".settings-body .skeleton").length).toBe(3);
    // 骨架期没有真实导航与徽标
    expect(screen.queryByRole("navigation", { name: "设置分区" })).not.toBeInTheDocument();
    expect(container.querySelectorAll(".settings-rail-badge").length).toBe(0);
  });

  it("loaded 之后换成真实 rail，骨架不再存在", async () => {
    const { container } = await mount();

    expect(container.querySelector(".settings-rail-skeleton")).toBeNull();
    expect(container.querySelector(".settings-shell")).not.toHaveAttribute("aria-busy");
    expect(screen.getByRole("navigation", { name: "设置分区" })).toBeInTheDocument();
  });
});

describe("Dashboard —— rail 结构与分组", () => {
  it("4 个分组标题按顺序出现，7 个条目分组归属正确", async () => {
    const { container } = await mount();
    const titles = Array.from(
      container.querySelectorAll(".settings-rail-title"),
    ).map((node) => node.textContent);
    expect(titles).toEqual(["接口与模型", "会话", "资料与行为", "其他"]);

    const items = Array.from(
      container.querySelectorAll(".settings-rail-item .settings-rail-name"),
    ).map((node) => node.textContent);
    expect(items).toEqual([
      "模型接口",
      "语音模型",
      "面试会话",
      "设备",
      "简历与项目资料",
      "回答行为",
      "保存与日志",
    ]);
  });

  it("glyph 是装饰性文本，条目标题落在 title 上", async () => {
    const { container } = await mount();
    const first = container.querySelector(".settings-rail-item") as HTMLElement;
    expect(first).toHaveAttribute("title", "模型接口");
    expect(first.querySelector(".settings-rail-glyph")).toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });
});

describe("Dashboard —— rail 徽标派生", () => {
  it("Key 未保存 → warn「Key 未保存」；已保存 → ok「Key 已保存」", async () => {
    hooks.settings = makeForm({ keySaved: false });
    const { unmount } = await mount();

    let badge = railBadge("模型接口") as HTMLElement;
    expect(badge.textContent).toBe("Key 未保存");
    expect(badge).toHaveClass("tone-warn");

    unmount();
    hooks.settings = makeForm({ keySaved: true });
    await mount();
    badge = railBadge("模型接口") as HTMLElement;
    expect(badge.textContent).toBe("Key 已保存");
    expect(badge).toHaveClass("tone-ok");
  });

  

  it.each([
    [{ phase: "ready" }, "就绪", "tone-ok"],
    [{ phase: "downloading", completedBytes: 120_678_628, totalBytes: 241_357_257 }, "50%", "tone-busy"],
    [{ phase: "loading" }, "读取中", "tone-busy"],
    [{ phase: "error", error: "读取失败" }, "失败", "tone-error"],
    [{ phase: "unprepared" }, "未下载", "tone-warn"],
  ])("语音模型 phase=%o → 「%s」 / %s", async (patch, text, tone) => {
    hooks.model = makeModel({ view: { ...INITIAL_MODEL_VIEW, ...patch } });
    await mount();

    const badge = railBadge("语音模型") as HTMLElement;
    expect(badge.textContent).toBe(text);
    expect(badge).toHaveClass(tone);
  });

  it.each([
    ["idle", "未开始", "tone-neutral"],
    ["connecting", "启动中", "tone-busy"],
    ["listening", "进行中", "tone-ok"],
    ["error", "失败", "tone-error"],
  ])("会话 state=%s → 「%s」 / %s", async (state, text, tone) => {
    hooks.session = makeSession({ state });
    await mount();

    const badge = railBadge("面试会话") as HTMLElement;
    expect(badge.textContent).toBe(text);
    expect(badge).toHaveClass(tone);
  });

  it("设备未指定时「系统默认」neutral", async () => {
    await mount();
    const badge = railBadge("设备") as HTMLElement;
    expect(badge.textContent).toBe("系统默认");
    expect(badge).toHaveClass("tone-neutral");
  });

  it("设备指定后显示已指定", async () => {
    hooks.settings = makeForm({
      form: { ...EMPTY_SETTINGS, micDevice: "mic-1" } as Settings,
    });
    await mount();

    const badge = railBadge("设备") as HTMLElement;
    expect(badge.textContent).toBe("已指定");
    expect(badge).toHaveClass("tone-ok");
  });

  it("简历与项目未填写时「未填写」warn", async () => {
    await mount();
    const badge = railBadge("简历与项目资料") as HTMLElement;
    expect(badge.textContent).toBe("未填写");
    expect(badge).toHaveClass("tone-warn");
  });

  it("简历与项目合计字数进徽标", async () => {
    hooks.settings = makeForm({
      form: {
        ...EMPTY_SETTINGS,
        resume: "一二三四五",
        projects: "六七八",
      } as Settings,
    });
    await mount();

    expect(railBadge("简历与项目资料")?.textContent).toBe("8 字");
  });

  it("触发方式 auto →「自动」，manual →「手动」", async () => {
    await mount();
    expect(railBadge("回答行为")?.textContent).toBe("自动");
  });

  it("触发方式 manual 时徽标为手动", async () => {
    hooks.settings = makeForm({
      form: { ...EMPTY_SETTINGS, triggerMode: "manual" } as Settings,
    });
    await mount();

    expect(railBadge("回答行为")?.textContent).toBe("手动");
  });

  it.each([
    ["配置已保存", "已保存", "tone-ok"],
    ["配置已保存，但当前接口还没有保存 API Key", "待处理", "tone-warn"],
    ["保存失败：网络错误", "失败", "tone-error"],
    ["正在保存", "提示", "tone-neutral"],
  ])("保存状态「%s」→ 徽标「%s」/ %s", async (status, text, tone) => {
    hooks.settings = makeForm({ status });
    await mount();

    const badge = railBadge("保存与日志") as HTMLElement;
    expect(badge.textContent).toBe(text);
    expect(badge).toHaveClass(tone);
  });

  it("status 为空时保存条目没有徽标", async () => {
    await mount();
    expect(railBadge("保存与日志")).toBeNull();
  });
});

describe("Dashboard —— rail 切换与 hidden 语义", () => {
  it("默认选中模型接口，只有 section-model 未 hidden", async () => {
    const { container } = await mount();

    expect(railItem("模型接口")).toHaveAttribute("aria-current", "true");
    expect(railItem("语音模型")).not.toHaveAttribute("aria-current");
    expect(visibleSections(container)).toEqual(["model"]);
    expect(railItem("模型接口")).toHaveAttribute("aria-controls", "section-model");
  });

  it("点击语音模型：active 与 aria-current 转移，只有 section-voice 可见", async () => {
    const user = userEvent.setup();
    const { container } = await mount();

    await user.click(railItem("语音模型"));

    expect(railItem("语音模型")).toHaveAttribute("aria-current", "true");
    expect(railItem("语音模型")).toHaveClass("active");
    expect(railItem("模型接口")).not.toHaveAttribute("aria-current");
    expect(visibleSections(container)).toEqual(["voice"]);
    // 标题栏跟着换
    expect(
      container.querySelector(".settings-title-name")?.textContent,
    ).toBe("语音模型");
  });

  it("七个条目逐个切换：每次恰好一个 section 可见且 id 对应", async () => {
    const user = userEvent.setup();
    const { container } = await mount();

    for (const [name, id] of [
      ["语音模型", "voice"],
      ["面试会话", "session"],
      ["设备", "devices"],
      ["简历与项目资料", "profile"],
      ["回答行为", "behavior"],
      ["保存与日志", "save"],
      ["模型接口", "model"],
    ]) {
      await user.click(railItem(name));
      expect(visibleSections(container)).toEqual([id]);
      expect(sectionOf(container, id).hidden).toBe(false);
    }
  });

  it("hidden 用属性收起而非卸载：隐藏的 section 仍在 DOM 里", async () => {
    const { container } = await mount();
    const voice = sectionOf(container, "voice");
    expect(voice).toBeInTheDocument();
    expect(voice.hidden).toBe(true);
    // 内容仍在（不重新挂载），只是被 hidden 收起
    expect(voice.querySelector("section.settings-block")).not.toBeNull();
  });
});

describe("Dashboard —— Tab 切换", () => {
  it("初始停在设置页：面板与 tab 选中态一致，复盘未启用", async () => {
    const { container } = await mount();

    expect(screen.getByRole("tab", { name: "设置" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "会话复盘" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    expect(container.querySelector("#tabpanel-settings")).not.toBeNull();
    expect(container.querySelector("#tabpanel-review")).toBeNull();
    expect(hooks.sessionsEnabled).toContain(false);
    expect(hooks.sessionsEnabled).not.toContain(true);
  });

  it("切到会话复盘：面板换成 SessionReview，useSessions 被启用", async () => {
    const user = userEvent.setup();
    const { container } = await mount();
    hooks.sessionsEnabled.length = 0;

    await user.click(screen.getByRole("tab", { name: "会话复盘" }));

    expect(container.querySelector("#tabpanel-settings")).toBeNull();
    expect(container.querySelector("#tabpanel-review")).not.toBeNull();
    expect(screen.getByRole("heading", { name: "历史会话" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "会话复盘" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(hooks.sessionsEnabled).toContain(true);
  });

  it("切回复盘再切回设置，设置面板恢复且复盘内容消失", async () => {
    const user = userEvent.setup();
    const { container } = await mount();

    await user.click(screen.getByRole("tab", { name: "会话复盘" }));
    await user.click(screen.getByRole("tab", { name: "设置" }));

    expect(container.querySelector("#tabpanel-review")).toBeNull();
    expect(screen.getByRole("navigation", { name: "设置分区" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "历史会话" })).not.toBeInTheDocument();
  });
});

describe("Dashboard —— banner 事件与自动消失", () => {
  it("hint://notice info → role=status、banner-info、aria-live=polite", async () => {
    await mount();
    emit("hint://notice", { level: "info", message: "模型已就绪" });

    const banner = screen.getByRole("status");
    expect(banner).toHaveTextContent("模型已就绪");
    expect(banner).toHaveClass("dashboard-banner", "banner-info");
    expect(banner).toHaveAttribute("aria-live", "polite");
    expect(banner).toHaveAttribute("title", "点击关闭");
  });

  it("level=error → banner-error 且 aria-live=assertive", async () => {
    await mount();
    emit("hint://notice", { level: "error", message: "热键注册失败" });

    const banner = screen.getByRole("status");
    expect(banner).toHaveClass("banner-error");
    expect(banner).toHaveAttribute("aria-live", "assertive");
  });

  it("未知 level 按 info 处理", async () => {
    await mount();
    emit("hint://notice", { level: "debug", message: "调试信息" });

    expect(screen.getByRole("status")).toHaveClass("banner-info");
  });

  it("空 message 或非对象 payload 不弹 banner", async () => {
    await mount();
    emit("hint://notice", { level: "info", message: "" });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();

    emit("hint://notice", "纯字符串");
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("点击 banner 关闭", async () => {
    const user = userEvent.setup();
    await mount();
    emit("hint://notice", { level: "info", message: "可关闭的提示" });

    await user.click(screen.getByRole("status"));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("info banner 6000ms 自动消失（5999ms 时仍在）", async () => {
    await mount();
    vi.useFakeTimers();
    emit("hint://notice", { level: "info", message: "6 秒后消失" });

    act(() => {
      vi.advanceTimersByTime(5999);
    });
    expect(screen.queryByRole("status")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("error banner 10000ms 才消失（9999ms 时仍在，比 info 长）", async () => {
    await mount();
    vi.useFakeTimers();
    emit("hint://notice", { level: "error", message: "10 秒后消失" });

    act(() => {
      vi.advanceTimersByTime(6000);
    });
    expect(screen.queryByRole("status")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(3999);
    });
    expect(screen.queryByRole("status")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("新 banner 覆盖旧 banner 并从该条重新计时", async () => {
    await mount();
    vi.useFakeTimers();
    emit("hint://notice", { level: "info", message: "第一条" });
    act(() => {
      vi.advanceTimersByTime(5000);
    });

    emit("hint://notice", { level: "error", message: "第二条" });
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    // 第一条的 6s 计时已作废，第二条是 10s
    expect(screen.getByRole("status")).toHaveTextContent("第二条");

    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("卸载后 hint://notice 不再写入 banner（监听被清理）", async () => {
    const { unmount } = await mount();
    expect(bus.handlers.get("hint://notice")?.length).toBe(1);

    unmount();
    expect(bus.handlers.get("hint://notice")?.length).toBe(0);
  });
});

describe("Dashboard —— 底部固定栏", () => {
  it("点「保存全部配置」调用 form.save 一次", async () => {
    const user = userEvent.setup();
    await mount();

    await user.click(screen.getByRole("button", { name: "保存全部配置" }));
    expect((hooks.settings as { save: unknown }).save).toHaveBeenCalledTimes(1);
  });

  it("form.action=save 时按钮文案变「保存中…」且禁用", async () => {
    hooks.settings = makeForm({ action: "save", busy: true });
    await mount();

    const button = screen.getByRole("button", { name: "保存中…" });
    expect(button).toBeDisabled();
  });

  it("busy 时保存按钮禁用但不改文案", async () => {
    hooks.settings = makeForm({ busy: true });
    await mount();

    expect(screen.getByRole("button", { name: "保存全部配置" })).toBeDisabled();
  });

  it("form.status 按语义色渲染：成功绿 / 警告黄 / 错误红 / 信息蓝", async () => {
    hooks.settings = makeForm({ status: "配置已保存" });
    const { container, unmount } = await mount();
    expect(container.querySelector(".settings-footer .status")?.textContent).toBe(
      "配置已保存",
    );
    unmount();

    hooks.settings = makeForm({ status: "配置已保存，但当前接口还没有保存 API Key" });
    const second = await mount();
    expect(
      second.container.querySelector(".settings-footer .status-warning")?.textContent,
    ).toContain("但当前接口还没有保存 API Key");
    second.unmount();

    hooks.settings = makeForm({ status: "保存失败：网络错误" });
    const third = await mount();
    expect(
      third.container.querySelector(".settings-footer .error-text")?.textContent,
    ).toBe("保存失败：网络错误");
    third.unmount();

    hooks.settings = makeForm({ status: "正在保存" });
    const fourth = await mount();
    expect(
      fourth.container.querySelector(".settings-footer .status-info")?.textContent,
    ).toBe("正在保存");
  });

  it("status 为空时不渲染状态行", async () => {
    const { container } = await mount();
    expect(container.querySelector(".settings-footer p")).toBeNull();
  });

  it("打开日志目录成功：调 open_log_dir 且不弹 banner", async () => {
    const user = userEvent.setup();
    await mount();

    await user.click(screen.getByRole("button", { name: "打开日志目录" }));

    expect(callsTo("open_log_dir")).toHaveLength(1);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("打开日志目录失败：banner 带原因", async () => {
    backend.routes.set("open_log_dir", () => Promise.reject("目录不存在"));
    const user = userEvent.setup();
    await mount();

    await user.click(screen.getByRole("button", { name: "打开日志目录" }));

    const banner = await screen.findByRole("status");
    expect(banner).toHaveTextContent("打开日志目录失败：目录不存在");
    expect(banner).toHaveClass("banner-error");
  });
});

describe("Dashboard —— 测试连接流转", () => {
  it("点「测试连接」走 form.testConnection，答案写入测试结果区", async () => {
    const testConnection = vi.fn(() =>
      Promise.resolve({ text: "我是模型，工作正常。", model: "deepseek-chat" }),
    );
    hooks.settings = makeForm({ testConnection });
    const user = userEvent.setup();
    await mount();

    // 先切到模型接口（默认就是），直接点
    await user.click(screen.getByRole("button", { name: "测试连接" }));

    expect(testConnection).toHaveBeenCalledTimes(1);
    const answer = await screen.findByLabelText("测试连接返回内容");
    expect(answer).toHaveTextContent("我是模型，工作正常。");
    expect(screen.getByText("连接成功")).toBeInTheDocument();
  });

  it("testConnection 返回 null：不写答案区，测试结论标连接失败", async () => {
    hooks.settings = makeForm({
      testConnection: vi.fn(() => Promise.resolve(null)),
      status: "连接失败：401",
    });
    const user = userEvent.setup();
    await mount();

    await user.click(screen.getByRole("button", { name: "测试连接" }));

    expect(screen.queryByLabelText("测试连接返回内容")).not.toBeInTheDocument();
    expect(screen.getByText("连接失败")).toBeInTheDocument();
  });
});

describe("Dashboard —— titlebar 技术值", () => {
  it("模型接口：协议短名 + 模型名", async () => {
    const { container } = await mount();
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "OpenAI 兼容 · deepseek-chat",
    );
  });

  it("模型名为空时写「未填模型」", async () => {
    hooks.settings = makeForm({ form: { ...EMPTY_SETTINGS, model: "" } as Settings });
    const { container } = await mount();
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "OpenAI 兼容 · 未填模型",
    );
  });

  it("语音模型 ready：已就绪 + 体积", async () => {
    const { container } = await mount();
    fireEvent.click(railItem("语音模型"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "已就绪 · 230.2 MB",
    );
  });

  it("语音模型 downloading：百分比 + 已完成/总量", async () => {
    hooks.model = makeModel({
      view: {
        ...INITIAL_MODEL_VIEW,
        phase: "downloading",
        completedBytes: 120_678_628,
        totalBytes: 241_357_257,
      },
    });
    const { container } = await mount();
    // 只有切到语音模型才渲染它的 meta
    fireEvent.click(railItem("语音模型"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "50.0% · 115.1 MB / 230.2 MB",
    );
  });

  it("语音模型未下载：总量 + 需下载", async () => {
    hooks.model = makeModel({
      view: { ...INITIAL_MODEL_VIEW, phase: "unprepared" },
    });
    await mount();
    fireEvent.click(railItem("语音模型"));
    expect(document.querySelector(".settings-meta")?.textContent).toBe(
      "230.2 MB · 需下载",
    );
  });

  it("面试会话：无会话时提示快捷键，有会话号时显示编号", async () => {
    const { container } = await mount();
    fireEvent.click(railItem("面试会话"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "未开始，快捷键 Ctrl+Shift+R",
    );
  });

  it("有会话号时 titlebar 显示编号", async () => {
    hooks.session = makeSession({ state: "listening", sessionId: 42, running: true });
    const { container } = await mount();
    fireEvent.click(railItem("面试会话"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe("会话 42");
  });

  it("会话状态胶囊只在面试会话区块出现", async () => {
    hooks.session = makeSession({ state: "listening", running: true });
    const { container } = await mount();
    // 默认模型区块没有胶囊
    expect(container.querySelector(".session-state")).toBeNull();

    fireEvent.click(railItem("面试会话"));
    const pill = container.querySelector(".session-state") as HTMLElement;
    expect(pill).toHaveClass("state-listening");
    expect(pill.textContent).toBe("面试进行中");
  });

  it("设备 / 资料 / 行为 / 保存 的 titlebar 值", async () => {
    hooks.settings = makeForm({
      form: {
        ...EMPTY_SETTINGS,
        resume: "五字简历",
        projects: "三字项",
        micDevice: "",
        outputDevice: "speaker-1",
      } as Settings,
    });
    const { container } = await mount();

    fireEvent.click(railItem("设备"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "麦克风 默认 · 输出 指定",
    );

    fireEvent.click(railItem("简历与项目资料"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "简历 4 字 · 项目 3 字",
    );

    fireEvent.click(railItem("回答行为"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "口语短答 · 自动触发 · 浮层 88%",
    );

    fireEvent.click(railItem("保存与日志"));
    expect(container.querySelector(".settings-meta")?.textContent).toBe(
      "logs\\hint.log",
    );
  });
});

describe("Dashboard —— 主题", () => {
  it("表单里的深色主题会落到根元素 data-theme 上", async () => {
    hooks.settings = makeForm({ form: { ...EMPTY_SETTINGS, theme: "dark" } });
    await mount();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("浅色主题不写 data-theme（默认状态即浅色）", async () => {
    hooks.settings = makeForm({ form: { ...EMPTY_SETTINGS, theme: "light" } });
    await mount();
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("表单值从浅色改成深色时配置页立即换肤（不用等保存）", async () => {
    const utils = await mount();
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
    // 模拟 useSettingsForm.patch：hook 返回值换成深色后再重渲染，
    // 等价于用户在「回答行为」里选了深色但还没点保存。
    hooks.settings = makeForm({ form: { ...EMPTY_SETTINGS, theme: "dark" } });
    await act(async () => {
      utils.rerender(<Dashboard />);
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});

describe("Dashboard —— 准备状态条", () => {
  it("缺项时列出还差哪几项", async () => {
    // 六个 rail 条目各显示自己的状态，但没有一处告诉用户「现在能不能开始面试」。
    // 后端 start_session 会拒（Key / 模型 / Base URL 与模型名），但用户在看到那条
    // 报错之前完全不知道还差几个条件。
    hooks.settings = makeForm({
      loaded: true,
      keySaved: false,
      form: { ...EMPTY_SETTINGS, baseUrl: "", model: "" },
    });
    hooks.model = makeModel({ view: { ...INITIAL_MODEL_VIEW, phase: "unprepared" } });
    await mount();

    expect(screen.getByText(/开始面试前还差 3 项/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Base URL 与模型名" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "API Key" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "下载模型" })).toBeInTheDocument();
  });

  it("点 chip 跳到对应区块", async () => {
    const user = userEvent.setup();
    hooks.settings = makeForm({
      loaded: true,
      keySaved: false,
      form: { ...EMPTY_SETTINGS, baseUrl: "", model: "" },
    });
    hooks.model = makeModel({ view: { ...INITIAL_MODEL_VIEW, phase: "unprepared" } });
    const { container } = await mount();

    await user.click(screen.getByRole("button", { name: "下载模型" }));
    expect(sectionOf(container, "voice").hidden).toBe(false);
  });

  it("全部齐备时不显示状态条（不占地方）", async () => {
    hooks.settings = makeForm({
      loaded: true,
      keySaved: true,
      form: {
        ...EMPTY_SETTINGS,
        baseUrl: "https://api.example.com/v1",
        model: "m",
      },
    });
    hooks.model = makeModel({ view: { ...INITIAL_MODEL_VIEW, phase: "ready" } });
    await mount();

    expect(screen.queryByText(/开始面试前还差/)).not.toBeInTheDocument();
  });
});
