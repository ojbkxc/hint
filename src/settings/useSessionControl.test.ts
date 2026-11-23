import { act, renderHook, waitFor } from "@testing-library/react";
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
  const state: { handler: (command: string) => Promise<unknown> } = {
    handler: () => Promise.resolve(null),
  };
  return { calls, state };
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
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    return backend.state.handler(command);
  }),
}));

import { useSessionControl } from "./useSessionControl";

const STATUS_EVENT = "hint://status";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function mount() {
  const utils = renderHook(() => useSessionControl());
  await waitFor(() => {
    expect(bus.registered).toContain(STATUS_EVENT);
  });
  return utils;
}

function emitStatus(payload: unknown) {
  act(() => {
    bus.emit(STATUS_EVENT, payload);
  });
}

/** hook 的 start/stop 返回 void（内部 fire-and-forget），只能刷微任务等状态落定。 */
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  bus.handlers.clear();
  bus.registered.length = 0;
  backend.calls.length = 0;
  backend.state.handler = () => Promise.resolve(null);
});

describe("useSessionControl —— 初始与状态事件", () => {
  it("初始是 idle、未运行、无忙碌动作", async () => {
    const { result } = await mount();
    expect(result.current.state).toBe("idle");
    expect(result.current.message).toBe("");
    expect(result.current.running).toBe(false);
    expect(result.current.busy).toBe(false);
    expect(result.current.busyAction).toBe("");
    expect(result.current.error).toBe("");
    expect(result.current.notice).toBe("");
    expect(result.current.sessionId).toBeNull();
  });

  it("listening 进入运行态、清错误，message 为空时给「面试进行中」", async () => {
    const { result } = await mount();
    emitStatus({ state: "error", message: "先失败一次" });
    expect(result.current.error).toBe("先失败一次");

    emitStatus({ state: "listening" });
    expect(result.current.state).toBe("listening");
    expect(result.current.running).toBe(true);
    expect(result.current.error).toBe("");
    expect(result.current.notice).toBe("面试进行中");
  });

  it("listening 带 message 时用后端文案做 notice", async () => {
    const { result } = await mount();
    emitStatus({ state: "listening", message: "正在监听系统声" });
    expect(result.current.notice).toBe("正在监听系统声");
  });

  it("idle 结束会话：notice 落到「会话已结束」并清掉 sessionId", async () => {
    backend.state.handler = () => Promise.resolve(7);
    const { result } = await mount();
    act(() => {
      result.current.start();
    });
    await flush();
    expect(result.current.sessionId).toBe(7);

    emitStatus({ state: "idle" });
    expect(result.current.state).toBe("idle");
    expect(result.current.running).toBe(false);
    expect(result.current.notice).toBe("会话已结束");
    expect(result.current.sessionId).toBeNull();
  });

  it("error 事件写进 error，无 message 时用「会话状态异常」", async () => {
    const { result } = await mount();
    emitStatus({ state: "error" });
    expect(result.current.state).toBe("error");
    expect(result.current.error).toBe("会话状态异常");
    emitStatus({ state: "error", message: "麦克风被占用" });
    expect(result.current.error).toBe("麦克风被占用");
  });

  it("connecting 处于运行态，message 进 notice；未知 state 归一成 idle", async () => {
    const { result } = await mount();
    emitStatus({ state: "connecting", message: "正在握手" });
    expect(result.current.state).toBe("connecting");
    expect(result.current.running).toBe(true);
    expect(result.current.notice).toBe("正在握手");

    emitStatus({ state: "weird-state", message: "不认识的状态" });
    expect(result.current.state).toBe("idle");
    expect(result.current.notice).toBe("不认识的状态");
  });

  it("缺 state 或非 record 的 payload 被忽略", async () => {
    const { result } = await mount();
    emitStatus({ message: "没有 state" });
    expect(result.current.notice).toBe("");
    emitStatus("not-a-record");
    expect(result.current.state).toBe("idle");
  });
});

describe("useSessionControl —— start", () => {
  it("命令未返回时 busyAction=start 且乐观进入 connecting", async () => {
    const pending = deferred<unknown>();
    backend.state.handler = () => pending.promise;
    const { result } = await mount();

    act(() => {
      result.current.start();
    });
    expect(result.current.busyAction).toBe("start");
    expect(result.current.busy).toBe(true);
    expect(result.current.state).toBe("connecting");
    expect(result.current.message).toContain("正在启动语音引擎");

    await act(async () => {
      pending.resolve(12);
    });
    await flush();
    expect(result.current.busyAction).toBe("");
    expect(result.current.sessionId).toBe(12);
    expect(result.current.notice).toBe("会话已启动");
    expect(result.current.state).toBe("connecting");
  });

  it("数字字符串形式的会话 id 也能落到 sessionId", async () => {
    backend.state.handler = () => Promise.resolve("34");
    const { result } = await mount();
    act(() => {
      result.current.start();
    });
    await flush();
    expect(result.current.sessionId).toBe(34);
  });

  it("非正数 / 非数字的返回值不写 sessionId", async () => {
    backend.state.handler = () => Promise.resolve(0);
    const { result } = await mount();
    act(() => {
      result.current.start();
    });
    await flush();
    expect(result.current.sessionId).toBeNull();

    backend.state.handler = () => Promise.resolve("abc");
    act(() => {
      result.current.start();
    });
    await flush();
    expect(result.current.sessionId).toBeNull();
  });

  it("失败时写 error 并把状态兜底成 error，busy 复位", async () => {
    backend.state.handler = () => Promise.reject("端口被占用");
    const { result } = await mount();
    act(() => {
      result.current.start();
    });
    await flush();
    expect(result.current.error).toBe("端口被占用");
    expect(result.current.state).toBe("error");
    expect(result.current.message).toBe("端口被占用");
    expect(result.current.busyAction).toBe("");
    expect(result.current.sessionId).toBeNull();
  });

  it("命令成功返回时会按代码现状把 error 状态改回 connecting（已知副作用，见报告）", async () => {
    const pending = deferred<unknown>();
    backend.state.handler = () => pending.promise;
    const { result } = await mount();

    act(() => {
      result.current.start();
    });
    emitStatus({ state: "error", message: "后端先报错" });
    expect(result.current.state).toBe("error");

    await act(async () => {
      pending.resolve(1);
    });
    await flush();
    // 成功分支只保护 listening，error 会被覆盖成 connecting（只保留 error 文案）
    expect(result.current.state).toBe("connecting");
    expect(result.current.error).toBe("后端先报错");
  });

  it("命令返回前已收到 listening 时保留 listening", async () => {
    const pending = deferred<unknown>();
    backend.state.handler = () => pending.promise;
    const { result } = await mount();

    act(() => {
      result.current.start();
    });
    emitStatus({ state: "listening", message: "正在监听" });

    await act(async () => {
      pending.resolve(5);
    });
    await flush();
    expect(result.current.state).toBe("listening");
    expect(result.current.running).toBe(true);
  });
});

describe("useSessionControl —— stop", () => {
  it("成功时回到 idle 并给出复盘提示", async () => {
    backend.state.handler = () => Promise.resolve(null);
    const { result } = await mount();
    act(() => {
      result.current.stop();
    });
    await flush();
    expect(result.current.state).toBe("idle");
    expect(result.current.running).toBe(false);
    expect(result.current.sessionId).toBeNull();
    expect(result.current.notice).toBe(
      "会话已结束，可在「会话复盘」里回看转录与问答。",
    );
    expect(result.current.busyAction).toBe("");
  });

  it("结束中 busyAction=stop", async () => {
    const pending = deferred<unknown>();
    backend.state.handler = () => pending.promise;
    const { result } = await mount();

    act(() => {
      result.current.stop();
    });
    expect(result.current.busyAction).toBe("stop");
    expect(result.current.busy).toBe(true);

    await act(async () => {
      pending.resolve(null);
    });
    await flush();
    expect(result.current.busyAction).toBe("");
  });

  it("失败时带「结束会话失败：」前缀，状态保持不动", async () => {
    backend.state.handler = () => Promise.reject("引擎无响应");
    const { result } = await mount();
    emitStatus({ state: "listening", message: "监听中" });

    act(() => {
      result.current.stop();
    });
    await flush();
    expect(result.current.error).toBe("结束会话失败：引擎无响应");
    // 状态没有被伪造成 idle
    expect(result.current.state).toBe("listening");
    expect(result.current.running).toBe(true);
    expect(result.current.busyAction).toBe("");
  });

  it("stop 会先清掉上一次的 error 与 notice", async () => {
    backend.state.handler = () => Promise.reject("第一次失败");
    const { result } = await mount();
    act(() => {
      result.current.stop();
    });
    await flush();
    expect(result.current.error).toBe("结束会话失败：第一次失败");

    backend.state.handler = () => Promise.resolve(null);
    act(() => {
      result.current.stop();
    });
    // 开始执行时旧的错误已被清空
    expect(result.current.error).toBe("");
    expect(result.current.notice).toBe("");
    await flush();
  });
});

describe("useSessionControl —— 卸载后不写状态", () => {
  it("命令返回前卸载，不再更新状态", async () => {
    const pending = deferred<unknown>();
    backend.state.handler = () => pending.promise;
    const { result, unmount } = await mount();

    act(() => {
      result.current.start();
    });
    unmount();
    await act(async () => {
      pending.resolve(3);
    });
    expect(result.current.state).toBe("connecting");
  });

  it("卸载时解绑 status 监听", async () => {
    const { unmount } = await mount();
    expect(bus.handlers.get(STATUS_EVENT)?.length).toBe(1);
    unmount();
    expect(bus.handlers.get(STATUS_EVENT)?.length).toBe(0);
  });
});
