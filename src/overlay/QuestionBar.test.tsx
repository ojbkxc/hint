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

import QuestionBar from "./QuestionBar";

const askCalls = () => backend.calls.filter((c) => c.command === "ask_question");

beforeEach(() => {
  backend.calls.length = 0;
  backend.state.handler = () => Promise.resolve(null);
});

describe("QuestionBar —— 浮层内手动提问", () => {
  it("回车提交问题到 ask_question，并清空输入", async () => {
    const user = userEvent.setup();
    render(<QuestionBar running />);
    const input = screen.getByLabelText("手动提问");

    await user.type(input, "请介绍一下你自己{Enter}");

    const asked = askCalls();
    expect(asked).toHaveLength(1);
    expect(asked[0].args).toEqual({ question: "请介绍一下你自己" });
    expect(input).toHaveValue("");
  });

  it("点「提问」按钮同样提交", async () => {
    const user = userEvent.setup();
    render(<QuestionBar running />);
    await user.type(screen.getByLabelText("手动提问"), "什么是回归测试");
    await user.click(screen.getByRole("button", { name: "提问" }));

    expect(askCalls()).toHaveLength(1);
    expect(askCalls()[0].args).toEqual({ question: "什么是回归测试" });
  });

  it("会话未运行时输入框与按钮都禁用，一个请求都不发", async () => {
    const user = userEvent.setup();
    render(<QuestionBar running={false} />);
    const input = screen.getByLabelText("手动提问");

    expect(input).toBeDisabled();
    expect(screen.getByRole("button", { name: "提问" })).toBeDisabled();
    expect(screen.getByPlaceholderText("开始面试后才能提问")).toBeInTheDocument();

    await user.type(input, "x{Enter}");
    expect(askCalls()).toHaveLength(0);
  });

  it("空白输入不发请求（避免白烧一次模型调用）", async () => {
    const user = userEvent.setup();
    render(<QuestionBar running />);
    const input = screen.getByLabelText("手动提问");
    await user.type(input, "   {Enter}");
    expect(askCalls()).toHaveLength(0);
  });

  it("输入框有内容时 Esc 只清空输入，不把 Escape 让给「隐藏浮层」", async () => {
    // 浮层的全局快捷键监听用 Esc 隐藏窗口。打字打到一半按 Esc 若直接隐藏浮层，
    // 用户会白输一遍——所以有内容时必须在输入框内吃掉这个事件。
    //
    // 判定点用「外层有没有收到 Escape」而不是「外层有没有被调用」：
    // user.type 每敲一个字符都会冒泡 keydown，比调用次数会误判。
    const user = userEvent.setup();
    const seen: string[] = [];
    render(
      <div onKeyDown={(e) => seen.push(e.key)}>
        <QuestionBar running />
      </div>,
    );
    const input = screen.getByLabelText("手动提问");
    await user.type(input, "半句话");
    await user.type(input, "{Escape}");

    expect(input).toHaveValue("");
    expect(seen).not.toContain("Escape");
  });

  it("输入已空时 Esc 放行，让浮层能正常隐藏", async () => {
    const user = userEvent.setup();
    const seen: string[] = [];
    render(
      <div onKeyDown={(e) => seen.push(e.key)}>
        <QuestionBar running />
      </div>,
    );
    await user.type(screen.getByLabelText("手动提问"), "{Escape}");
    expect(seen).toContain("Escape");
  });

  it("后端报错时把原因显示出来", async () => {
    const user = userEvent.setup();
    backend.state.handler = () => Promise.reject("还没有开始面试会话");
    render(<QuestionBar running />);
    await user.type(screen.getByLabelText("手动提问"), "问题{Enter}");

    expect(await screen.findByText("还没有开始面试会话")).toBeInTheDocument();
  });
});

describe("QuestionBar —— 方向键滚动答案区", () => {
  /** 造一个可滚动的答案区，并记录 scrollBy 调用。 */
  function mountWithScroller() {
    const calls: Array<{ top: number }> = [];
    const scroller = document.createElement("div");
    scroller.className = "overlay-answer-body";
    Object.defineProperty(scroller, "clientHeight", { value: 200, configurable: true });
    scroller.scrollBy = ((opts: { top?: number }) => {
      calls.push({ top: opts?.top ?? 0 });
    }) as typeof scroller.scrollBy;
    document.body.appendChild(scroller);

    const utils = render(<QuestionBar running />);
    return { ...utils, calls, cleanupDom: () => scroller.remove() };
  }

  it("输入框为空时方向键滚动答案区（焦点在输入框也不挡）", async () => {
    // 输入框在会话开始时自动聚焦；焦点在 <input> 上时方向键只会移动光标，
    // 不会滚动下方答案区。面试里答案常 150 字、浮层高度有限，往上翻看前文
    // 是常见动作，每次都要先点一下答案区很打断节奏。
    const user = userEvent.setup();
    const { calls, cleanupDom } = mountWithScroller();

    await user.type(screen.getByLabelText("手动提问"), "{ArrowDown}");
    expect(calls).toHaveLength(1);
    expect(calls[0].top).toBeGreaterThan(0);

    await user.type(screen.getByLabelText("手动提问"), "{ArrowUp}");
    expect(calls).toHaveLength(2);
    expect(calls[1].top).toBeLessThan(0);

    cleanupDom();
  });

  it("PageDown 一次翻约一屏", async () => {
    const user = userEvent.setup();
    const { calls, cleanupDom } = mountWithScroller();
    await user.type(screen.getByLabelText("手动提问"), "{PageDown}");
    expect(calls).toHaveLength(1);
    // clientHeight 200 → 约 180
    expect(calls[0].top).toBeGreaterThan(100);
    cleanupDom();
  });

  it("输入框有内容时方向键不抢滚动（保持光标移动）", async () => {
    const user = userEvent.setup();
    const { calls, cleanupDom } = mountWithScroller();
    const input = screen.getByLabelText("手动提问");
    await user.type(input, "abc");
    calls.length = 0;
    await user.type(input, "{ArrowDown}");
    expect(calls).toHaveLength(0);
    cleanupDom();
  });
});
