import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AnswerPanel from "./AnswerPanel";
import type { AnswerState } from "./types";

function makeAnswer(overrides: Partial<AnswerState> = {}): AnswerState {
  return {
    text: "",
    streaming: false,
    done: false,
    model: "",
    configuredModel: "m",
    ttftMs: null,
    question: "",
    pending: 0,
    error: "",
    retryable: false,
    muted: false,
    ...overrides,
  };
}

function panel(answer: AnswerState) {
  return (
    <AnswerPanel
      answer={answer}
      history={[]}
      answeredTotal={0}
      onRegenerate={vi.fn()}
      onClear={vi.fn()}
      onDismissError={vi.fn()}
    />
  );
}

/**
 * 推进定时器。
 *
 * 必须包 `act`：`useWaitSeconds` 里 `setInterval` 的回调会 setState，不 flush 的话
 * 组件不会重渲染——第一版没包，测试看到的一直是初始文案「正在等待模型输出…」，
 * 看起来像功能没生效。
 */
async function tick(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
  });
}

beforeEach(() => {
  vi.useFakeTimers();
});

describe("AnswerPanel —— 慢响应提示", () => {
  it("刚开始等待时不显示秒数，也不显示「较慢」", () => {
    render(panel(makeAnswer({ streaming: true })));
    const node = screen.getByText(/正在等待模型输出/);
    expect(node.textContent).not.toMatch(/已等待/);
    expect(screen.queryByText(/响应较慢/)).toBeNull();
  });

  it("等过 5 秒显示已等待秒数，但还不说「较慢」", async () => {
    render(panel(makeAnswer({ streaming: true })));
    await tick(5000);
    expect(screen.getByText(/正在等待模型输出/).textContent).toMatch(/已等待 5 秒/);
    expect(screen.queryByText(/响应较慢/)).toBeNull();
  });

  it("等过 10 秒提示「响应较慢」并给出可行动建议", async () => {
    render(panel(makeAnswer({ streaming: true })));
    await tick(11000);
    const node = screen.getByText(/正在等待模型输出/);
    expect(node.textContent).toMatch(/已等待 11 秒/);
    expect(node.textContent).toMatch(/响应较慢/);
    // 要给出下一步动作，而不是只报「慢」
    expect(node.textContent).toMatch(/重新生成/);
  });

  it("首字到达后提示消失", async () => {
    const { rerender } = render(panel(makeAnswer({ streaming: true })));
    await tick(12000);
    expect(screen.getByText(/响应较慢/)).toBeInTheDocument();

    rerender(panel(makeAnswer({ streaming: true, text: "回归测试是…" })));
    expect(screen.queryByText(/响应较慢/)).toBeNull();
    expect(screen.getByText(/回归测试是/)).toBeInTheDocument();
  });

  it("不在流式状态时不显示任何等待文案", () => {
    render(panel(makeAnswer({ streaming: false, text: "" })));
    expect(screen.queryByText(/正在等待模型输出/)).toBeNull();
  });

  it("等超过 10 秒后换上 slow-hint 样式（可被样式区分）", async () => {
    render(panel(makeAnswer({ streaming: true })));
    await tick(11000);
    expect(screen.getByText(/正在等待模型输出/).className).toContain("slow-hint");
  });

  it("10 秒边界：9 秒时不出现「较慢」，10 秒时出现", async () => {
    render(panel(makeAnswer({ streaming: true })));
    await tick(9000);
    expect(screen.queryByText(/响应较慢/)).toBeNull();
    await tick(1000);
    expect(screen.getByText(/响应较慢/)).toBeInTheDocument();
  });
});
