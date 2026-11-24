import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

type InvokeHandler = (command: string, args: unknown) => Promise<unknown>;
const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const state: { handler: InvokeHandler } = {
    handler: () => Promise.resolve(null),
  };
  return { calls, state };
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    return backend.state.handler(command, args);
  }),
}));

import SessionCard, { SESSION_STATE_LABEL } from "./SessionCard";
import type { SessionControl } from "./useSessionControl";

function makeControl(overrides: Partial<SessionControl> = {}): SessionControl {
  const start = vi.fn();
  const stop = vi.fn();
  return {
    state: "idle",
    message: "",
    running: false,
    busy: false,
    busyAction: "",
    error: "",
    notice: "",
    sessionId: null,
    start,
    stop,
    ...overrides,
  };
}

/** 直接渲染卡片并返回 command 调用记录。 */
function renderCard(overrides: Partial<SessionControl> = {}) {
  const utils = render(<SessionCard control={makeControl(overrides)} />);
  return {
    ...utils,
    askCalls: () => backend.calls.filter((c) => c.command === "ask_question"),
  };
}

beforeEach(() => {
  backend.calls.length = 0;
  backend.state.handler = () => Promise.resolve(null);
});

describe("SessionCard —— 状态文案四分支", () => {
  it("idle：未开始 / 设置页空闲 / 会话编号为破折号", () => {
    render(<SessionCard control={makeControl()} />);
    const kv = screen.getByText("当前状态").parentElement as HTMLElement;
    expect(kv.textContent).toContain("未开始");
    expect(screen.getByText("设置页空闲")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("connecting：进入启动态，显示状态名与首次启动说明", () => {
    render(
      <SessionCard
        control={makeControl({ state: "connecting", message: "正在启动语音引擎", running: true })}
      />,
    );
    const kv = screen.getByText("当前状态").parentElement as HTMLElement;
    expect(kv.textContent).toContain("正在启动语音引擎…");
    expect(screen.getByText(/首次启动可能需要 10–30 秒/)).toBeInTheDocument();
    expect(screen.getByText("浮层已接管")).toBeInTheDocument();
    // connecting 期间不重复展示「当前状态：…」段落
    expect(screen.queryByText(/当前状态：正在启动语音引擎/)).not.toBeInTheDocument();
  });

  it("listening：进行中，显示会话编号", () => {
    render(
      <SessionCard
        control={makeControl({
          state: "listening",
          running: true,
          sessionId: 42,
          message: "正在监听系统声",
        })}
      />,
    );
    const kv = screen.getByText("当前状态").parentElement as HTMLElement;
    expect(kv.textContent).toContain("面试进行中");
    expect(kv.textContent).toContain("42");
    expect(screen.getByText("本次会话编号：42")).toBeInTheDocument();
    expect(screen.getByText("当前状态：正在监听系统声")).toBeInTheDocument();
  });

  it("error：状态名「启动失败」，error 详情与排查提示都出现，notice 被压掉", () => {
    render(
      <SessionCard
        control={makeControl({
          state: "error",
          error: "端口被占用",
          notice: "这条不该出现",
        })}
      />,
    );
    const kv = screen.getByText("当前状态").parentElement as HTMLElement;
    expect(kv.textContent).toContain("启动失败");
    expect(screen.getByText("端口被占用")).toHaveClass("error-text");
    expect(screen.getByText(/可检查「模型接口」的 API Key/)).toBeInTheDocument();
    expect(screen.queryByText("这条不该出现")).not.toBeInTheDocument();
  });

  it("未知状态直接显示原始值", () => {
    render(<SessionCard control={makeControl({ state: "weird" as SessionControl["state"] })} />);
    const kv = screen.getByText("当前状态").parentElement as HTMLElement;
    expect(kv.textContent).toContain("weird");
  });

  it("状态映射表覆盖后端会发的六个状态", () => {
    expect(SESSION_STATE_LABEL).toEqual({
      idle: "未开始",
      connecting: "正在启动语音引擎…",
      listening: "面试进行中",
      ready: "已就绪",
      connected: "已连接",
      error: "启动失败",
    });
  });
});

describe("SessionCard —— 开始/结束按钮", () => {
  it("idle：开始可点、结束禁用，点击分别走 control.start / control.stop", async () => {
    const user = userEvent.setup();
    const control = makeControl();
    render(<SessionCard control={control} />);

    const start = screen.getByRole("button", { name: "开始面试" });
    const stop = screen.getByRole("button", { name: "结束面试" });
    expect(start).toBeEnabled();
    expect(stop).toBeDisabled();

    await user.click(start);
    expect(control.start).toHaveBeenCalledTimes(1);
  });

  it("connecting：开始按钮文案变「正在启动…」且禁用，结束按钮此时可点", () => {
    render(<SessionCard control={makeControl({ state: "connecting", running: true })} />);
    expect(screen.getByRole("button", { name: "正在启动…" })).toBeDisabled();
    // running 为真，结束按钮不禁用（用户可以在卡住时主动结束）
    expect(screen.getByRole("button", { name: "结束面试" })).toBeEnabled();
  });

  it("listening：开始禁用、结束可点", async () => {
    const user = userEvent.setup();
    const control = makeControl({ state: "listening", running: true });
    render(<SessionCard control={control} />);

    expect(screen.getByRole("button", { name: "开始面试" })).toBeDisabled();
    const stop = screen.getByRole("button", { name: "结束面试" });
    expect(stop).toBeEnabled();
    await user.click(stop);
    expect(control.stop).toHaveBeenCalledTimes(1);
  });

  it("busy（非启动中）：两个按钮都禁用，开始文案为「启动中…」", () => {
    render(<SessionCard control={makeControl({ busy: true, busyAction: "start" })} />);
    expect(screen.getByRole("button", { name: "启动中…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "结束面试" })).toBeDisabled();
  });

  it("busyAction=stop：结束按钮文案变「正在结束…」", () => {
    render(
      <SessionCard control={makeControl({ busy: true, busyAction: "stop", running: true })} />,
    );
    expect(screen.getByRole("button", { name: "正在结束…" })).toBeDisabled();
  });

  it("error 后开始按钮改为「重新开始面试」", () => {
    render(<SessionCard control={makeControl({ state: "error", error: "启动失败" })} />);
    expect(screen.getByRole("button", { name: "重新开始面试" })).toBeEnabled();
  });
});

describe("SessionCard —— 提示段落", () => {
  it("notice 走 status-info，error 存在时不显示 notice", () => {
    const ok = render(
      <SessionCard control={makeControl({ notice: "会话已结束，可在「会话复盘」里回看转录与问答。" })} />,
    );
    expect(
      ok.getByText("会话已结束，可在「会话复盘」里回看转录与问答。"),
    ).toHaveClass("status-info");
    ok.unmount();

    render(
      <SessionCard control={makeControl({ notice: "提示", error: "错误" })} />,
    );
    expect(screen.queryByText("提示")).not.toBeInTheDocument();
  });

  it("message 为空不渲染「当前状态：」段落；非 listening 时不显示会话编号", () => {
    render(<SessionCard control={makeControl({ state: "idle", sessionId: 9 })} />);
    expect(screen.queryByText(/当前状态：/)).not.toBeInTheDocument();
    expect(screen.queryByText(/本次会话编号/)).not.toBeInTheDocument();
    // 但状态来源表里仍能看到编号
    const kv = screen.getByText("会话编号").parentElement as HTMLElement;
    expect(kv.textContent).toContain("9");
  });

  it("无 error / notice 时两块提示都不渲染", () => {
    const { container } = render(<SessionCard control={makeControl()} />);
    expect(container.querySelector(".error-text")).toBeNull();
    expect(container.querySelector(".status-info")).toBeNull();
  });
});

describe("SessionCard —— 手动提问（绕开语音识别的兜底）", () => {
  it("会话进行中可以向 ask_question 提交问题", async () => {
    // 这条路径的意义：面试作答原本只能由 STT 识别出的问题驱动，
    // 识别不准时用户就完全没法让产品产出答案。ask_question 直接调模型，
    // 不经过识别——模型与流式链路本身是好的（截图问答走同一份 stream_chat）。
    const user = userEvent.setup();
    const { askCalls } = renderCard({ state: "listening", running: true });

    const input = screen.getByLabelText("要回答的问题");
    await user.type(input, "请介绍一下你自己");
    await user.click(screen.getByRole("button", { name: "提问" }));

    const asked = askCalls();
    expect(asked).toHaveLength(1);
    expect(asked[0].args).toEqual({ question: "请介绍一下你自己" });
  });

  it("没开始面试时输入框与按钮都不可用", () => {
    renderCard({ state: "idle", running: false });
    expect(screen.getByLabelText("要回答的问题")).toBeDisabled();
    expect(screen.getByRole("button", { name: "提问" })).toBeDisabled();
    expect(screen.getByText("开始面试后才能提问。")).toBeInTheDocument();
  });

  it("空白问题不发请求（避免白烧一次模型调用）", async () => {
    const user = userEvent.setup();
    const { askCalls } = renderCard({ state: "listening", running: true });
    const input = screen.getByLabelText("要回答的问题");
    await user.type(input, "   ");
    // 按钮在空白输入时是禁用的，直接回车也应被前端拦住
    await user.type(input, "{Enter}");
    expect(askCalls()).toHaveLength(0);
  });
});
