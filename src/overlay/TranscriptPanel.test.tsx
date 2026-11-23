import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * 事件总线 mock：用于「超过 6 行截断到最新」的集成用例。
 * 截断发生在 useOverlayEvents（TranscriptPanel 本身是纯展示组件），
 * 只测组件拿不到这条行为，所以这里把 hook + 组件一起渲染。
 */
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

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (event: string, handler: (incoming: { payload: unknown }) => void) => {
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
    },
  ),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

import TranscriptPanel from "./TranscriptPanel";
import { useOverlayEvents } from "../lib/useOverlayEvents";
import { HINT_EVENTS, MAX_TRANSCRIPT_LINES, type TranscriptLine } from "./types";

function makeLine(overrides: Partial<TranscriptLine> = {}): TranscriptLine {
  return {
    id: 1,
    channel: "interviewer",
    text: "请介绍一下你自己",
    isEcho: false,
    ...overrides,
  };
}

/** 用 hook 的真实状态驱动组件，覆盖「截断后仍然只显示最新 6 行」。 */
function HookHarness() {
  const { transcript } = useOverlayEvents();
  return <TranscriptPanel lines={transcript} />;
}

async function mountHarness() {
  const utils = render(<HookHarness />);
  await waitFor(() => {
    expect(
      bus.registered.filter((name) => name.startsWith("hint://")).length,
    ).toBe(Object.keys(HINT_EVENTS).length);
  });
  return utils;
}

function emit(event: string, payload: unknown) {
  act(() => {
    bus.emit(event, payload);
  });
}

beforeEach(() => {
  bus.handlers.clear();
  bus.registered.length = 0;
});

describe("TranscriptPanel —— 空态", () => {
  it("没有转录时显示引导文案并保持可聚焦的 log 语义", () => {
    const { container } = render(<TranscriptPanel lines={[]} />);
    const root = container.querySelector(".overlay-transcript");
    expect(root).toHaveAttribute("role", "log");
    expect(root).toHaveAttribute("aria-label", "实时转录");
    expect(root).toHaveAttribute("tabindex", "0");
    expect(screen.getByText("麦克风与系统声通道的转录会出现在这里。")).toBeInTheDocument();
    expect(container.querySelectorAll(".line")).toHaveLength(0);
  });
});

describe("TranscriptPanel —— 双通道行", () => {
  it("面试官与我分别打上对应类名与中文标签", () => {
    const { container } = render(
      <TranscriptPanel
        lines={[
          makeLine({ id: 1, channel: "interviewer", text: "你为什么离职？" }),
          makeLine({ id: 2, channel: "me", text: "因为想换方向" }),
        ]}
      />,
    );
    const rows = Array.from(container.querySelectorAll(".line"));
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveClass("line", "interviewer");
    expect(rows[0].querySelector(".who")).toHaveTextContent("面试官");
    expect(rows[0].querySelector(".what")).toHaveTextContent("你为什么离职？");
    expect(rows[1]).toHaveClass("line", "me");
    expect(rows[1].querySelector(".who")).toHaveTextContent("我");
    expect(rows[1].querySelector(".what")).toHaveTextContent("因为想换方向");
    // 空态文案不应与有内容状态共存
    expect(screen.queryByText(/麦克风与系统声通道的转录会出现在这里/)).not.toBeInTheDocument();
  });

  it("按传入顺序渲染（最新在最后），空文本行也不丢", () => {
    const { container } = render(
      <TranscriptPanel
        lines={[
          makeLine({ id: 1, text: "第一句" }),
          makeLine({ id: 2, text: "" }),
          makeLine({ id: 3, text: "第三句" }),
        ]}
      />,
    );
    const whats = Array.from(container.querySelectorAll(".line .what")).map(
      (node) => node.textContent,
    );
    expect(whats).toEqual(["第一句", "", "第三句"]);
  });
});

describe("TranscriptPanel —— 回声与补录标记", () => {
  it("回声行加 echo 类并显示「回声」标签", () => {
    const { container } = render(
      <TranscriptPanel
        lines={[makeLine({ id: 1, channel: "me", text: "（重复的我的话）", isEcho: true })]}
      />,
    );
    const row = container.querySelector(".line");
    expect(row).toHaveClass("line", "me", "echo");
    const tag = row?.querySelector(".echo-tag");
    expect(tag).toHaveTextContent("回声");
    expect(row?.querySelector(".recovered-tag")).toBeNull();
  });

  it("补录行单独显示「补录」标签，且不带 echo 类", () => {
    const { container } = render(
      <TranscriptPanel
        lines={[makeLine({ id: 1, text: "断线期间的转录", recovered: true })]}
      />,
    );
    const row = container.querySelector(".line");
    expect(row).not.toHaveClass("echo");
    expect(row?.querySelector(".recovered-tag")).toHaveTextContent("补录");
    expect(row?.querySelector(".echo-tag")).toBeNull();
  });

  it("回声 + 补录同时成立时两个标签都在", () => {
    const { container } = render(
      <TranscriptPanel
        lines={[makeLine({ id: 1, isEcho: true, recovered: true, text: "两者都有" })]}
      />,
    );
    const row = container.querySelector(".line");
    expect(row?.querySelector(".echo-tag")).not.toBeNull();
    expect(row?.querySelector(".recovered-tag")).not.toBeNull();
    expect(row).toHaveClass("echo");
  });

  it("普通行两个标记都不渲染", () => {
    const { container } = render(<TranscriptPanel lines={[makeLine()]} />);
    const row = container.querySelector(".line");
    expect(row?.querySelector(".echo-tag")).toBeNull();
    expect(row?.querySelector(".recovered-tag")).toBeNull();
  });
});

describe("TranscriptPanel —— 超过 MAX_TRANSCRIPT_LINES 截断到最新", () => {
  it("连续 8 行转录只保留最新 6 行，最早的先被挤掉", async () => {
    await mountHarness();
    for (let i = 1; i <= 8; i += 1) {
      emit(HINT_EVENTS.transcript, {
        channel: i % 2 === 0 ? "me" : "interviewer",
        text: `行${i}`,
      });
    }
    const rows = Array.from(document.querySelectorAll(".overlay-transcript .line"));
    expect(rows).toHaveLength(MAX_TRANSCRIPT_LINES);
    expect(MAX_TRANSCRIPT_LINES).toBe(6);
    expect(rows[0].querySelector(".what")).toHaveTextContent("行3");
    expect(rows[5].querySelector(".what")).toHaveTextContent("行8");
    // 被截断的两行确实不在了
    expect(screen.queryByText("行1")).not.toBeInTheDocument();
    expect(screen.queryByText("行2")).not.toBeInTheDocument();
  });

  it("截断后新行继续追加，窗口始终是最后 6 行", async () => {
    await mountHarness();
    for (let i = 1; i <= 7; i += 1) {
      emit(HINT_EVENTS.transcript, { channel: "me", text: `行${i}` });
    }
    expect(document.querySelectorAll(".overlay-transcript .line")).toHaveLength(6);
    emit(HINT_EVENTS.transcript, { channel: "me", text: "行8", recovered: true });
    const rows = Array.from(document.querySelectorAll(".overlay-transcript .line"));
    expect(rows).toHaveLength(6);
    expect(rows[0].querySelector(".what")).toHaveTextContent("行3");
    // 最后一条带补录标记，说明标记随行一起进窗口
    expect(rows[5].querySelector(".recovered-tag")).not.toBeNull();
  });
});