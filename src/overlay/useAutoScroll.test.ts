import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useRef } from "react";
import { useAutoScroll } from "./useAutoScroll";
import type { AnswerState } from "./types";

function makeAnswer(overrides: Partial<AnswerState> = {}): AnswerState {
  return {
    text: "",
    streaming: false,
    done: false,
    model: "",
  configuredModel: "",
    ttftMs: null,
    question: "",
    pending: 0,
    error: "",
    retryable: false,
    muted: false,
    ...overrides,
  };
}

function setup(initial: AnswerState = makeAnswer()) {
  const node = document.createElement("div");
  const host = document.createElement("div");
  host.appendChild(node);
  document.body.appendChild(host);

  const utils = renderHook(
    ({ answer }: { answer: AnswerState }) => {
      const ref = useRef<HTMLDivElement | null>(node);
      return useAutoScroll(ref, answer);
    },
    { initialProps: { answer: initial } },
  );

  return { node, ...utils, cleanupNode: () => document.body.removeChild(host) };
}

/** 模拟用户把容器滚到某位置后触发的 scroll 回调（组件把它绑在 onScroll 上）。 */
function userScroll(node: HTMLElement, result: { current: { onScroll: () => void } }, top: number) {
  node.scrollTop = top;
  act(() => {
    result.current.onScroll();
  });
}

describe("useAutoScroll —— 跟随最新答案", () => {
  it("新一轮有文本时把已滚下去的容器拉回顶部", () => {
    const { node, rerender, cleanupNode } = setup();
    node.scrollTop = 300;

    rerender({ answer: makeAnswer({ text: "第一轮答案", question: "问题一" }) });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });

  it("流式追加时保持贴顶", () => {
    const { node, rerender, cleanupNode } = setup(
      makeAnswer({ text: "第一", question: "问题一", streaming: true }),
    );
    node.scrollTop = 120;
    rerender({ answer: makeAnswer({ text: "第一句", question: "问题一", streaming: true }) });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });

  it("文本变短视为新一轮，重新贴顶", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "很长的一段答案内容", question: "问题一" }),
    );
    userScroll(node, result, 500);
    // 同问题、文本变短 → newRound
    rerender({ answer: makeAnswer({ text: "短", question: "问题一" }) });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });

  it("问题变化视为新一轮，重新贴顶", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "答案", question: "问题一" }),
    );
    userScroll(node, result, 500);
    rerender({ answer: makeAnswer({ text: "答案", question: "问题二" }) });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });
});

describe("useAutoScroll —— 手动上翻后停止跟随", () => {
  it("scrollTop >= 24 后新增内容不再抢回顶部", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "第一句", question: "问题一", streaming: true }),
    );
    userScroll(node, result, 200);
    rerender({
      answer: makeAnswer({ text: "第一句继续增长", question: "问题一", streaming: true }),
    });
    expect(node.scrollTop).toBe(200);
    cleanupNode();
  });

  it("scrollTop 恰好 24 时停止跟随（边界含 24）", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "第一句", question: "问题一", streaming: true }),
    );
    userScroll(node, result, 24);
    rerender({
      answer: makeAnswer({ text: "第一句继续增长", question: "问题一", streaming: true }),
    });
    expect(node.scrollTop).toBe(24);
    cleanupNode();
  });

  it("scrollTop 23 仍算贴顶，继续跟随", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "第一句", question: "问题一", streaming: true }),
    );
    userScroll(node, result, 23);
    rerender({
      answer: makeAnswer({ text: "第一句继续增长", question: "问题一", streaming: true }),
    });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });

  it("上翻后回滚到顶部（<24）恢复跟随", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "第一句", question: "问题一", streaming: true }),
    );
    userScroll(node, result, 300);
    userScroll(node, result, 10);
    rerender({
      answer: makeAnswer({ text: "第一句继续增长", question: "问题一", streaming: true }),
    });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });

  it("停止跟随后问题变化会重新开始跟随", () => {
    const { node, result, rerender, cleanupNode } = setup(
      makeAnswer({ text: "答案", question: "问题一" }),
    );
    userScroll(node, result, 400);
    rerender({ answer: makeAnswer({ text: "答案", question: "问题二" }) });
    expect(node.scrollTop).toBe(0);
    cleanupNode();
  });

  it("onScroll 在没有节点时不抛异常", () => {
    const { result } = renderHook(() => {
      const ref = useRef<HTMLDivElement | null>(null);
      return useAutoScroll(ref, makeAnswer());
    });
    expect(() => result.current.onScroll()).not.toThrow();
  });
});
