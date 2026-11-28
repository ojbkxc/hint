import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const bus = vi.hoisted(() => {
  type Handler = (payload: unknown) => void;
  const handlers = new Map<string, Handler[]>();
  const registered: string[] = [];
  return {
    handlers,
    registered,
    emit(event: string, payload: unknown) {
      for (const handler of handlers.get(event) ?? []) handler(payload);
    },
  };
});

const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const routes = new Map<string, (args: unknown) => unknown>();
  return { calls, routes };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (incoming: { payload: unknown }) => void) => {
    bus.registered.push(event);
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

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    const handler = backend.routes.get(command);
    return Promise.resolve().then(() => (handler ? handler(args) : null));
  }),
}));

import Overlay from "./Overlay";
import { HINT_EVENTS } from "./overlay/types";

/** 等浮层把所有事件都订阅上（9 个后端事件）。 */
async function mount() {
  const utils = render(<Overlay />);
  await waitFor(() => {
    expect(bus.handlers.get(HINT_EVENTS.status)?.length).toBe(1);
  });
  return utils;
}

function emit(event: string, payload: unknown) {
  act(() => {
    bus.emit(event, payload);
  });
}

function callsTo(command: string) {
  return backend.calls.filter((call) => call.command === command);
}

beforeEach(() => {
  bus.handlers.clear();
  bus.registered.length = 0;
  backend.calls.length = 0;
  backend.routes.clear();
});

describe("Overlay —— 头部连接状态", () => {
  it("初始为「等待连接」且样式为 tone-idle", async () => {
    const { container } = await mount();
    const status = container.querySelector(".overlay-status") as HTMLElement;
    expect(status.textContent).toContain("等待连接");
    expect(status).toHaveClass("tone-idle");
  });

  it.each([
    ["listening", "监听中", "tone-ok"],
    ["connected", "已连接", "tone-ok"],
    ["ready", "已就绪", "tone-ok"],
    ["connecting", "连接中", "tone-pending"],
    ["error", "连接异常", "tone-error"],
    ["idle", "等待连接", "tone-idle"],
  ])("status=%s → 文案「%s」+ %s", async (state, label, tone) => {
    const { container } = await mount();
    emit(HINT_EVENTS.status, { state, message: "" });
    const status = container.querySelector(".overlay-status") as HTMLElement;
    expect(status.textContent).toContain(label);
    expect(status).toHaveClass(tone);
  });

  it("后端带 message 时优先显示 message，title 同样是 message", async () => {
    const { container } = await mount();
    emit(HINT_EVENTS.status, { state: "listening", message: "正在监听系统声" });
    const status = container.querySelector(".overlay-status") as HTMLElement;
    expect(status.textContent).toContain("正在监听系统声");
    expect(status).toHaveAttribute("title", "正在监听系统声");
  });

  it("未知状态值原样显示，神态回落 tone-idle", async () => {
    const { container } = await mount();
    emit(HINT_EVENTS.status, { state: "weird", message: "" });
    const status = container.querySelector(".overlay-status") as HTMLElement;
    expect(status.textContent).toContain("weird");
    expect(status).toHaveClass("tone-idle");
  });

  it("有首字延迟时头部追加「首字 N.Ns」", async () => {
    const { container } = await mount();
    emit(HINT_EVENTS.answerDone, { text: "答案", model: "m1", ttftMs: 2400 });
    await waitFor(() => {
      expect(container.querySelector(".header-latency")?.textContent).toBe("首字 2.4s");
    });
    // StatusBar 同一份数据也显示
    expect(screen.getAllByText("首字 2.4s").length).toBeGreaterThan(0);
  });
});

describe("Overlay —— Esc 隐藏浮层", () => {
  it("按 Esc 调 hide_overlay", async () => {
    await mount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => {
      expect(callsTo("hide_overlay")).toHaveLength(1);
    });
  });

  it("Esc 重复按键（event.repeat）只发一次命令", async () => {
    await mount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", repeat: true }));
    });
    expect(callsTo("hide_overlay")).toHaveLength(0);

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => {
      expect(callsTo("hide_overlay")).toHaveLength(1);
    });
  });

  it("非 Esc 键不触发隐藏", async () => {
    await mount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    });
    expect(callsTo("hide_overlay")).toHaveLength(0);
  });

  it("hide_overlay 失败时弹错误 toast，可点击关闭", async () => {
    backend.routes.set("hide_overlay", () => Promise.reject("窗口已销毁"));
    const user = userEvent.setup();
    await mount();

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("隐藏浮层失败：窗口已销毁");

    await user.click(alert);
    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  it("卸载后移除 Esc 监听", async () => {
    const { unmount } = await mount();
    unmount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(callsTo("hide_overlay")).toHaveLength(0);
  });
});

describe("Overlay —— 重新生成与清空", () => {
  it("点「重新生成」调 regenerate_answer 并切到流式", async () => {
    const user = userEvent.setup();
    await mount();

    await user.click(screen.getByRole("button", { name: "重新生成" }));
    await waitFor(() => {
      expect(callsTo("regenerate_answer")).toHaveLength(1);
    });
    expect(screen.getByText("正在等待模型输出…")).toBeInTheDocument();
  });

  it("regenerate_answer 失败时错误条显示「重新生成失败：…」", async () => {
    backend.routes.set("regenerate_answer", () => Promise.reject("队列已满"));
    const user = userEvent.setup();
    await mount();

    await user.click(screen.getByRole("button", { name: "重新生成" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("重新生成失败：队列已满");
  });

  it("清空按钮只清显示：答案文本消失，但排队数保留", async () => {
    const user = userEvent.setup();
    await mount();

    emit(HINT_EVENTS.answerDelta, {
      text: "一段答案",
      reset: true,
      question: "自我介绍",
      pending: 2,
    });
    expect(screen.getByText("一段答案")).toBeInTheDocument();
    expect(screen.getByText("排队 2")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "清空" }));

    expect(screen.queryByText("一段答案")).not.toBeInTheDocument();
    expect(screen.getByText("排队 2")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "清空" })).not.toBeInTheDocument();
  });

  it("答案错误条由后端 error 驱动，「关闭」只关提示保留文本", async () => {
    const user = userEvent.setup();
    await mount();

    emit(HINT_EVENTS.answerDelta, { text: "半截答案", reset: true, question: "q" });
    emit(HINT_EVENTS.answerDone, {
      text: "半截答案",
      model: "m1",
      error: "模型超时",
      retryable: true,
    });

    const errorBar = await screen.findByRole("alert");
    expect(errorBar).toHaveTextContent("模型超时");
    expect(screen.getByRole("button", { name: "重试" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "关闭" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByText("半截答案")).toBeInTheDocument();
  });
});

describe("Overlay —— 电平跳动不影响骨架", () => {
  it("电平事件只更新状态条，答案面板 DOM 节点不动（memo 生效）", async () => {
    const { container } = await mount();
    emit(HINT_EVENTS.answerDelta, { text: "稳定答案", reset: true, question: "q" });

    const answerNode = screen.getByText("稳定答案");
    const transcriptNode = container.querySelector(".overlay-transcript");
    const answerPanelNode = container.querySelector(".overlay-answer");
    const regenerateBefore = screen.getByRole("button", { name: "重新生成" });

    for (let index = 1; index <= 5; index += 1) {
      emit(HINT_EVENTS.levels, { mic: index / 10, system: (10 - index) / 10 });
    }

    const mic = container.querySelector(".level-mic") as HTMLElement;
    expect(mic).toHaveAttribute("title", "麦克风电平 50%");
    expect(answerNode).toBe(screen.getByText("稳定答案"));
    expect(container.querySelector(".overlay-transcript")).toBe(transcriptNode);
    expect(container.querySelector(".overlay-answer")).toBe(answerPanelNode);
    expect(screen.getByRole("button", { name: "重新生成" })).toBe(regenerateBefore);
  });

  it("电平值只接受 0–1：越界被 clamp", async () => {
    const { container } = await mount();
    emit(HINT_EVENTS.levels, { mic: 4, system: -2 });

    expect(container.querySelector(".level-mic")).toHaveAttribute("title", "麦克风电平 100%");
    expect(container.querySelector(".level-system")).toHaveAttribute("title", "系统声电平 0%");
  });

  it("说话人徽标跟随 speech-edge 事件切换", async () => {
    const { container } = await mount();
    emit(HINT_EVENTS.speechEdge, { joint: "interviewer" });
    expect(container.querySelector(".speaking-badge")?.textContent).toBe("对方在说");

    emit(HINT_EVENTS.speechEdge, { joint: "neither" });
    expect(container.querySelector(".speaking-badge")).toBeNull();
  });
});

describe("Overlay —— 后端提示 toast", () => {
  it("notice 事件升级为 info toast，role=status", async () => {
    await mount();
    emit(HINT_EVENTS.notice, { level: "info", message: "模型已就绪" });
    const toast = await screen.findByRole("status");
    expect(toast).toHaveTextContent("模型已就绪");
    expect(toast).toHaveClass("toast-info");
  });

  it("notice level=error 走 role=alert 与 toast-error", async () => {
    await mount();
    emit(HINT_EVENTS.notice, { level: "error", message: "热键注册失败" });
    const toast = await screen.findByRole("alert");
    expect(toast).toHaveTextContent("热键注册失败");
    expect(toast).toHaveClass("toast-error");
  });

  it("空 message 的 notice 不弹 toast", async () => {
    await mount();
    emit(HINT_EVENTS.notice, { level: "info", message: "" });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("点击 toast 可关闭", async () => {
    const user = userEvent.setup();
    await mount();
    emit(HINT_EVENTS.notice, { level: "info", message: "可关闭的提示" });
    const toast = await screen.findByRole("status");

    await user.click(toast);
    await waitFor(() => {
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    });
  });
});

describe("Overlay —— 启动时读取不透明度", () => {
  it("get_settings 成功时把 overlayOpacity 写到 CSS 变量", async () => {
    backend.routes.set("get_settings", () => ({ overlayOpacity: 0.5 }));
    await mount();
    await waitFor(() => {
      expect(
        document.documentElement.style.getPropertyValue("--overlay-alpha"),
      ).toBe("0.5");
    });
  });

  it("get_settings 失败时保持默认 CSS，不弹错误", async () => {
    backend.routes.set("get_settings", () => Promise.reject("读取失败"));
    await mount();
    expect(document.documentElement.style.getPropertyValue("--overlay-alpha")).toBe("");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("overlay-opacity 事件即时更新 CSS 变量并 clamp", async () => {
    await mount();
    emit(HINT_EVENTS.overlayOpacity, { opacity: 0.42 });
    expect(document.documentElement.style.getPropertyValue("--overlay-alpha")).toBe("0.42");

    emit(HINT_EVENTS.overlayOpacity, { value: 9 });
    expect(document.documentElement.style.getPropertyValue("--overlay-alpha")).toBe("1");
  });

  /**
   * 不透明度只作用于**浮层背景**，文字本身必须保持不透明。
   *
   * ## 为什么值得锁
   *
   * 真机验证过：不透明度调到最低（20%）时，背景 `rgba(255,255,255,0.2)` 是半透
   * 明的，但文字颜色仍是 `rgb(20,22,26)`——完全不透明。这正是 20% 下限仍可用的
   * 原因：用户调透明是为了看清下方视频画面，**不是为了让文字也变透明**。
   *
   * 若将来有人把文字也加上 alpha（"让文字也跟随透明度"），低不透明度下文字就会
   * 变透明、看不清。这条测试钉住这个设计。
   */
  it("调低不透明度时文字颜色保持不透明（透明度只作用于背景）", async () => {
    backend.routes.set("get_settings", () => ({ overlayOpacity: 0.2 }));
    await mount();
    await waitFor(() => {
      expect(
        document.documentElement.style.getPropertyValue("--overlay-alpha"),
      ).toBe("0.2");
    });

    // 找到答案文字元素，检查其颜色不含透明通道
    const answerEl = document.querySelector(".answer-text, .answer-current");
    expect(answerEl).not.toBeNull();
    const color = window.getComputedStyle(answerEl as Element).color;

    // 颜色不能是 rgba(...,0) 或 transparent
    const isTransparent =
      color.includes("rgba(0, 0, 0, 0)") ||
      color.toLowerCase() === "transparent" ||
      /rgba\([^)]*,\s*0\s*\)/.test(color);
    expect(isTransparent).toBe(false);
    expect(color).toContain("rgb");
  });
});