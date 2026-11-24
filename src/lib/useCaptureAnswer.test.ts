import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * 伪造的 Tauri 事件总线（与 useOverlayEvents.test.ts 同一套写法）：
 * vi.hoisted 保证 mock 工厂（会被提升到文件顶部）能引用到它。
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

const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  /** 需要模拟失败的命令（后端未注册 / 调用报错）。 */
  const failing: string[] = [];
  return { calls, failing };
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
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    if (backend.failing.includes(command)) return Promise.reject("命令未注册");
    return Promise.resolve(null);
  }),
}));

import { CAPTURE_ANSWER_EVENT, useCaptureAnswer } from "./useCaptureAnswer";

/** 把一次 emit 包进 act，让 React 状态更新同步落定。 */
function emit(payload: unknown) {
  act(() => {
    bus.emit(CAPTURE_ANSWER_EVENT, payload);
  });
}

function listeners() {
  return bus.handlers.get(CAPTURE_ANSWER_EVENT) ?? [];
}

/** 等 safeListen 的异步注册完成再断言。 */
async function mount() {
  const utils = renderHook(() => useCaptureAnswer());
  await waitFor(() => expect(listeners()).toHaveLength(1));
  return utils;
}

beforeEach(() => {
  bus.handlers.clear();
  bus.registered.length = 0;
  backend.calls.length = 0;
  backend.failing.length = 0;
});

describe("useCaptureAnswer —— 初始态", () => {
  it("默认全空：没有内容时不显示截图答案区", async () => {
    const { result } = await mount();
    expect(result.current.text).toBe("");
    expect(result.current.streaming).toBe(false);
    expect(result.current.error).toBe("");
    expect(result.current.model).toBe("");
  });

  it("只订阅截图问答这一个事件，不碰面试答案事件", async () => {
    await mount();
    expect(bus.registered).toEqual([CAPTURE_ANSWER_EVENT]);
  });
});

describe("useCaptureAnswer —— 增量拼接", () => {
  it("reset 帧打底，后续增量按顺序追加，streaming 保持 true", async () => {
    const { result } = await mount();

    emit({ text: "这张图是", reset: true, done: false });
    expect(result.current.text).toBe("这张图是");
    expect(result.current.streaming).toBe(true);

    emit({ text: "一道", reset: false, done: false });
    emit({ text: "算法题", reset: false, done: false });

    expect(result.current.text).toBe("这张图是一道算法题");
    expect(result.current.streaming).toBe(true);
  });

  it("text 缺失或为非字符串时按空处理，不产生 NaN/undefined 文本", async () => {
    const { result } = await mount();

    emit({ text: "开头", reset: true });
    emit({ reset: false, done: false });

    expect(result.current.text).toBe("开头");
    expect(result.current.streaming).toBe(true);
  });

  it("非记录型 payload（null / 数组 / 字符串）被忽略", async () => {
    const { result } = await mount();

    emit({ text: "有效", reset: true });
    emit(null);
    emit([1, 2, 3]);
    emit("文本");

    expect(result.current.text).toBe("有效");
  });
});

describe("useCaptureAnswer —— reset 清空", () => {
  it("reset=true 整段替换旧答案，并清掉上一轮的错误", async () => {
    const { result } = await mount();

    emit({ text: "上一张图的答案", reset: true });
    emit({ error: "截图编码失败", done: true });
    expect(result.current.error).toBe("截图编码失败");

    emit({ text: "新答案", reset: true, done: false });

    expect(result.current.text).toBe("新答案");
    expect(result.current.error).toBe("");
    expect(result.current.streaming).toBe(true);
  });
});

describe("useCaptureAnswer —— done 收尾", () => {
  it("done=true 结束流式，保留文本与模型名", async () => {
    const { result } = await mount();

    emit({ text: "完整答案", reset: true, done: false, model: "gpt-4o-mini" });
    // 结束帧按后端契约不带 text：带上会被当成增量拼到上一段后面。
    emit({ text: "", reset: false, done: true, model: "gpt-4o-mini" });

    expect(result.current.text).toBe("完整答案");
    expect(result.current.streaming).toBe(false);
    expect(result.current.model).toBe("gpt-4o-mini");
  });

  it("reset 与 done 同帧到达：直接结束，不闪一次流式光标", async () => {
    const { result } = await mount();

    emit({ text: "一次性答案", reset: true, done: true, model: "qwen-vl" });

    expect(result.current.text).toBe("一次性答案");
    expect(result.current.streaming).toBe(false);
    expect(result.current.model).toBe("qwen-vl");
  });

  it("后续帧不带 model 时保留上一次的模型名", async () => {
    const { result } = await mount();

    emit({ text: "a", reset: true, model: "m1" });
    emit({ text: "b", reset: false, done: true });

    expect(result.current.model).toBe("m1");
  });
});

describe("useCaptureAnswer —— error 上屏", () => {
  it("error 帧把错误写进状态、停止流式，并保留已收到的半截文本", async () => {
    const { result } = await mount();

    emit({ text: "回答了半", reset: true });
    emit({ error: "截图问答超时", done: true, model: "m1" });

    expect(result.current.error).toBe("截图问答超时");
    expect(result.current.streaming).toBe(false);
    expect(result.current.text).toBe("回答了半");
    expect(result.current.model).toBe("m1");
  });

  it("error 优先于同帧的 done/文本：不会被当成正常收尾而清掉", async () => {
    const { result } = await mount();

    emit({ text: "部分内容", reset: true, done: true, error: "模型不可用" });

    expect(result.current.error).toBe("模型不可用");
    expect(result.current.text).toBe("部分内容");
    expect(result.current.streaming).toBe(false);
  });
});

describe("useCaptureAnswer —— dismiss 与卸载", () => {
  it("dismiss 清空本地状态并尽力调 cancel_capture_answer", async () => {
    const { result } = await mount();

    emit({ text: "答案", reset: true, done: true, model: "m1" });
    act(() => {
      result.current.dismiss();
    });

    expect(result.current.text).toBe("");
    expect(result.current.streaming).toBe(false);
    expect(result.current.error).toBe("");
    expect(result.current.model).toBe("");
    // dismiss 里的 safeInvoke 是异步的（要先过 isTauriAsync），必须等一拍。
    await waitFor(() =>
      expect(backend.calls.map((call) => call.command)).toContain(
        "cancel_capture_answer",
      ),
    );
  });

  it("cancel_capture_answer 失败（后端未注册）不抛错、不影响清空", async () => {
    backend.failing.push("cancel_capture_answer");
    const { result } = await mount();
    emit({ text: "答案", reset: true, done: true });

    act(() => {
      result.current.dismiss();
    });

    expect(result.current.text).toBe("");
    await waitFor(() => {
      expect(backend.calls.map((call) => call.command)).toContain(
        "cancel_capture_answer",
      );
    });
    // 失败不能把错误贴到界面上（截图答案区只认后端 downlink 的 error）。
    expect(result.current.error).toBe("");
  });

  it("卸载时解绑监听：事件总线里不再留下 handler", async () => {
    const { unmount } = await mount();
    expect(listeners()).toHaveLength(1);

    unmount();

    expect(listeners()).toHaveLength(0);
    // 卸载后再 emit 也不应抛错（disposed 分支直接返回）。
    expect(() => emit({ text: "迟到的一帧", reset: true })).not.toThrow();
  });
});
