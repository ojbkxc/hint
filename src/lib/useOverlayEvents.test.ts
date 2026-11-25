import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * 伪造的 Tauri 事件总线。
 * vi.hoisted 保证 mock 工厂（会被提升到文件顶部）能引用到它。
 */
const bus = vi.hoisted(() => {
  type Handler = (payload: unknown) => void;
  const handlers = new Map<string, Handler[]>();
  const registered: string[] = [];
  return {
    handlers,
    registered,
    /** 后端 emit 一个事件 */
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
        const next = (bus.handlers.get(event) ?? []).filter(
          (item) => item !== wrapped,
        );
        bus.handlers.set(event, next);
      });
    },
  ),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string) => {
    // useOverlayEvents 启动时读 get_settings；给一个空对象走默认不透明度。
    if (command === "get_settings") return Promise.resolve({});
    return Promise.resolve(null);
  }),
}));

import { useOverlayEvents } from "./useOverlayEvents";
import { HINT_EVENTS } from "../overlay/types";

/** 把一次 emit 包进 act，让 React 状态更新同步完成。 */
function emit(event: string, payload: unknown) {
  act(() => {
    bus.emit(event, payload);
  });
}

/** 等所有 safeListen 注册完成（它们是异步 Promise）。 */
async function mount() {
  const utils = renderHook(() => useOverlayEvents());
  await waitFor(() => {
    // 9 个事件全部注册完才算挂载完成
    expect(bus.registered.filter((n) => n.startsWith("hint://")).length).toBe(
      Object.keys(HINT_EVENTS).length,
    );
  });
  return utils;
}

/** 发一帧带 reset 的 answer-delta，代表新一轮开始。 */
function startAnswer(text: string, question: string) {
  emit(HINT_EVENTS.answerDelta, { reset: true, text, question, pending: 0 });
}

beforeEach(() => {
  bus.handlers.clear();
  bus.registered.length = 0;
});

describe("useOverlayEvents —— 流式中清空后增量不上屏（1.0.9 修复）", () => {
  it("clearAnswer 后剩余增量与 answer-done 全文都不再上屏，muted 恒为 true", async () => {
    const { result } = await mount();

    startAnswer("第一", "问题一");
    emit(HINT_EVENTS.answerDelta, { text: "句" });
    expect(result.current.answer.text).toBe("第一句");
    expect(result.current.answer.muted).toBe(false);

    // 流式进行中点「清空」
    act(() => {
      result.current.clearAnswer();
    });
    expect(result.current.answer.text).toBe("");
    expect(result.current.answer.muted).toBe(true);

    // 后续增量不能从半句处重新长出来
    emit(HINT_EVENTS.answerDelta, { text: "继续流式" });
    expect(result.current.answer.text).toBe("");
    expect(result.current.answer.muted).toBe(true);

    // answer-done 也不能把最终全文贴回来
    emit(HINT_EVENTS.answerDone, { text: "第一句继续流式完整全文", model: "m" });
    expect(result.current.answer.text).toBe("");
    expect(result.current.answer.muted).toBe(true);
    expect(result.current.answer.done).toBe(true);
    expect(result.current.answer.model).toBe("m");
  });

  it("清空后错误帧照样提示错误，但正文保持空", async () => {
    const { result } = await mount();
    startAnswer("片段", "问题");
    act(() => {
      result.current.clearAnswer();
    });
    emit(HINT_EVENTS.answerDone, { error: "网络中断", retryable: true });
    expect(result.current.answer.text).toBe("");
    expect(result.current.answer.error).toBe("网络中断");
    expect(result.current.answer.retryable).toBe(true);
    expect(result.current.answer.muted).toBe(true);
  });

  it("流式结束后再清空不会置 muted（下一轮 reset 才重新上屏）", async () => {
    const { result } = await mount();
    startAnswer("已完成的", "问题");
    emit(HINT_EVENTS.answerDone, { text: "已完成的答案", model: "m" });
    expect(result.current.answer.done).toBe(true);

    act(() => {
      result.current.clearAnswer();
    });
    expect(result.current.answer.muted).toBe(false);

    // 新一轮 reset：内容恢复上屏
    startAnswer("新", "新问题");
    expect(result.current.answer.text).toBe("新");
    expect(result.current.answer.muted).toBe(false);
  });
});

describe("useOverlayEvents —— 归档 id 单调递增且 answeredTotal 不封顶（1.0.9 修复）", () => {
  it("answerHistory 超 10 条被截断，answeredTotal 继续增长，id 严格递增", async () => {
    const { result } = await mount();

    // 归档发生在「下一轮 reset」时，所以发 13 轮只会归档前 12 轮。
    const ROUNDS = 13;
    for (let i = 1; i <= ROUNDS; i += 1) {
      startAnswer(`第${i}轮答案`, `问题${i}`);
      emit(HINT_EVENTS.answerDone, { text: `第${i}轮答案`, model: "m" });
    }

    // 数组截断到 MAX_ANSWER_HISTORY=10，但独立计数器拿到了 12（不被 10 封顶）
    expect(result.current.answerHistory.length).toBe(10);
    expect(result.current.answeredTotal).toBe(12);
    expect(result.current.answeredTotal).toBeGreaterThan(10);

    // 窗口内 id 依旧严格单调递增（用作渲染 key），且不因截断回退
    const ids = result.current.answerHistory.map((entry) => entry.id);
    for (let i = 1; i < ids.length; i += 1) {
      expect(ids[i]).toBe(ids[i - 1] + 1);
    }
    // 最老一条是第 3 轮，最新一条是第 12 轮
    expect(ids[0]).toBe(3);
    expect(ids[ids.length - 1]).toBe(12);
    expect(result.current.answerHistory[0].question).toBe("问题3");
    expect(result.current.answerHistory[9].text).toBe("第12轮答案");
  });

  it("每轮回答完成后下一轮 reset 才归档，同轮的 done 不提前归档", async () => {
    const { result } = await mount();
    startAnswer("第一轮", "问题一");
    emit(HINT_EVENTS.answerDone, { text: "第一轮完整", model: "m" });
    // done 之后、下一轮 reset 之前：还没有归档
    expect(result.current.answerHistory.length).toBe(0);
    expect(result.current.answeredTotal).toBe(0);

    startAnswer("第二轮", "问题二");
    expect(result.current.answerHistory.length).toBe(1);
    expect(result.current.answerHistory[0]).toEqual({
      id: 1,
      question: "问题一",
      text: "第一轮完整",
      model: "m",
    });
    expect(result.current.answeredTotal).toBe(1);
  });

  it("空问题或空文本的上一轮不会被归档，answeredTotal 不虚增", async () => {
    const { result } = await mount();
    // 上一轮 question 为空（reset 帧里没带 question 且没有转录）
    startAnswer("有文本", "");
    emit(HINT_EVENTS.answerDone, { text: "有文本完整", model: "m" });
    startAnswer("下一轮", "问题二");
    expect(result.current.answerHistory.length).toBe(0);
    expect(result.current.answeredTotal).toBe(0);
  });
});

describe("useOverlayEvents —— clearAnswer 不抹掉排队数", () => {
  it("队列 pending=3 时清空后 pending 保持 3", async () => {
    const { result } = await mount();
    emit(HINT_EVENTS.queue, { pending: 3 });
    expect(result.current.answer.pending).toBe(3);

    act(() => {
      result.current.clearAnswer();
    });
    expect(result.current.answer.text).toBe("");
    // 后端队列还在，排队数不能被抹成 0
    expect(result.current.answer.pending).toBe(3);
  });

  it("清空后新的队列事件继续生效（1 → 0）", async () => {
    const { result } = await mount();
    emit(HINT_EVENTS.queue, { pending: 1 });
    act(() => {
      result.current.clearAnswer();
    });
    emit(HINT_EVENTS.queue, { pending: 0 });
    expect(result.current.answer.pending).toBe(0);
  });

  it("流式中的 muted 分支仍会更新排队数", async () => {
    const { result } = await mount();
    startAnswer("片段", "问题");
    act(() => {
      result.current.clearAnswer();
    });
    // muted 状态下收到带 pending 的增量：文本不上屏，排队数更新
    emit(HINT_EVENTS.answerDelta, { text: "不该上屏", pending: 2 });
    expect(result.current.answer.text).toBe("");
    expect(result.current.answer.pending).toBe(2);
  });

  it("queue 事件带负数或小数时钳成非负整数", async () => {
    const { result } = await mount();
    emit(HINT_EVENTS.queue, { pending: -1 });
    expect(result.current.answer.pending).toBe(0);
    emit(HINT_EVENTS.queue, { pending: 2.7 });
    expect(result.current.answer.pending).toBe(2);
    emit(HINT_EVENTS.queue, { pending: "bad" });
    expect(result.current.answer.pending).toBe(2);
  });
});

describe("useOverlayEvents —— 其他事件路径", () => {
  it("dismissAnswerError 只关错误提示，保留已生成的正文", async () => {
    const { result } = await mount();
    startAnswer("半句", "问题");
    emit(HINT_EVENTS.answerDone, {
      text: "半句+完整",
      model: "m",
      error: "超时",
      retryable: true,
    });
    expect(result.current.answer.text).toBe("半句+完整");
    expect(result.current.answer.error).toBe("超时");

    act(() => {
      result.current.dismissAnswerError();
    });
    expect(result.current.answer.text).toBe("半句+完整");
    expect(result.current.answer.error).toBe("");
    expect(result.current.answer.retryable).toBe(false);
    expect(result.current.answer.done).toBe(true);
  });

  it("hideOverlay / regenerate 走 safeInvoke 并返回结果", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const invokeMock = vi.mocked(invoke);

    const { result } = await mount();
    // once 队列要在 mount 之后设置：mount 期间的 get_settings 会先消耗
    // 排在最前面的 once 值。
    invokeMock.mockResolvedValueOnce(null); // hide_overlay 成功
    invokeMock.mockRejectedValueOnce("命令失败"); // regenerate 失败

    expect(await result.current.hideOverlay()).toBeNull();

    const error = await result.current.regenerate();
    expect(error).toBe("命令失败");
    // 失败时 regenerate 直接返回错误，不动状态（不假装进入流式）
    expect(result.current.answer.streaming).toBe(false);

    // 成功路径才切到流式，且不清空已上屏的正文
    startAnswer("已有正文", "问题");
    emit(HINT_EVENTS.answerDone, { text: "已有正文", model: "m" });
    invokeMock.mockResolvedValueOnce(null);
    await act(async () => {
      expect(await result.current.regenerate()).toBeNull();
    });
    expect(result.current.answer.streaming).toBe(true);
    expect(result.current.answer.done).toBe(false);
    expect(result.current.answer.text).toBe("已有正文");
  });

  it("get_settings 启动读取成功时把不透明度写进 CSS 变量", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke).mockResolvedValueOnce({ overlayOpacity: 0.5 });
    const { result } = await mount();
    await waitFor(() => {
      expect(
        document.documentElement.style.getPropertyValue("--overlay-alpha"),
      ).toBe("0.5");
    });
    // 同时不应影响其他状态
    expect(result.current.answer.text).toBe("");
  });

  it("overlayOpacity 事件即时更新 CSS 变量（钳位）", async () => {
    await mount();
    emit(HINT_EVENTS.overlayOpacity, { opacity: 2 });
    expect(
      document.documentElement.style.getPropertyValue("--overlay-alpha"),
    ).toBe("1");
    emit(HINT_EVENTS.overlayOpacity, { value: "0.1" });
    expect(
      document.documentElement.style.getPropertyValue("--overlay-alpha"),
    ).toBe("0.2");
  });

  it("get_settings 带回深色主题时根元素写上 data-theme=dark", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke).mockResolvedValueOnce({ theme: "dark" });
    await mount();
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("dark");
    });
  });

  it("overlayTheme 事件切换主题；非法值按浅色处理（属性被删掉）", async () => {
    await mount();
    emit(HINT_EVENTS.overlayTheme, { theme: "dark" });
    expect(document.documentElement.dataset.theme).toBe("dark");
    emit(HINT_EVENTS.overlayTheme, { theme: "light" });
    expect(document.documentElement.dataset.theme).toBeUndefined();
    // 未知值不能把浮层停在半黑状态：一律回落浅色。
    emit(HINT_EVENTS.overlayTheme, { theme: "dark" });
    emit(HINT_EVENTS.overlayTheme, { theme: "午夜蓝" });
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("speechEdge 事件只接受四个合法 joint 值", async () => {
    const { result } = await mount();
    emit(HINT_EVENTS.speechEdge, { joint: "interviewer" });
    expect(result.current.speech.joint).toBe("interviewer");
    emit(HINT_EVENTS.speechEdge, { joint: "both" });
    expect(result.current.speech.joint).toBe("both");
    emit(HINT_EVENTS.speechEdge, { joint: "不合法的值" });
    expect(result.current.speech.joint).toBe("both");
  });

  it("transcript 事件按 channel 归类并截断到 6 条", async () => {
    const { result } = await mount();
    for (let i = 1; i <= 8; i += 1) {
      emit(HINT_EVENTS.transcript, {
        channel: i % 2 === 0 ? "me" : "interviewer",
        text: `行${i}`,
      });
    }
    expect(result.current.transcript.length).toBe(6);
    expect(result.current.transcript[0].text).toBe("行3");
    expect(result.current.transcript[5]).toEqual({
      id: 8,
      channel: "me",
      text: "行8",
      isEcho: false,
      recovered: false,
    });
  });

  it("reset 帧缺 question 时用最近的非回声提问行兜底", async () => {
    const { result } = await mount();
    emit(HINT_EVENTS.transcript, {
      channel: "interviewer",
      text: "请介绍你的项目",
    });
    emit(HINT_EVENTS.transcript, {
      channel: "interviewer",
      text: "（我插的话）",
      isEcho: true,
    });
    startAnswer("答", "");
    expect(result.current.answer.question).toBe("请介绍你的项目");
  });

  it("status idle / connecting 清空转录与历史计数", async () => {
    const { result } = await mount();
    startAnswer("答", "问");
    emit(HINT_EVENTS.answerDone, { text: "答完整", model: "m" });
    startAnswer("答2", "问2");
    expect(result.current.answeredTotal).toBe(1);

    emit(HINT_EVENTS.status, { state: "idle", message: "结束" });
    expect(result.current.transcript).toEqual([]);
    expect(result.current.answerHistory).toEqual([]);
    expect(result.current.answeredTotal).toBe(0);
    expect(result.current.answer.text).toBe("");
  });

  it("levels 事件钳到 0–1，notice 事件缺省 level 用 info", async () => {
    const { result } = await mount();
    emit(HINT_EVENTS.levels, { mic: 1.7, system: -0.5 });
    expect(result.current.levels).toEqual({ mic: 1, system: 0 });

    emit(HINT_EVENTS.notice, { message: "模型下载中" });
    expect(result.current.notice).toEqual({ level: "info", message: "模型下载中" });
  });

  it("非 record 的 payload 被静默忽略，不抛异常", async () => {
    const { result } = await mount();
    const before = result.current.answer.text;
    emit(HINT_EVENTS.answerDelta, "not-a-record");
    emit(HINT_EVENTS.answerDone, null);
    emit(HINT_EVENTS.levels, 42);
    expect(result.current.answer.text).toBe(before);
  });
});
