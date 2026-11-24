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
  const calls: string[] = [];
  const state: { handler: (command: string) => Promise<unknown> } = {
    handler: () => Promise.resolve({ phase: "unprepared" }),
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
  invoke: vi.fn((command: string) => {
    backend.calls.push(command);
    return backend.state.handler(command);
  }),
}));

import { useModelManager } from "./useModelManager";
import { DEFAULT_TOTAL_BYTES } from "./modelTypes";

const MODEL_EVENT = "hint://model-progress";

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
  const utils = renderHook(() => useModelManager());
  await waitFor(() => {
    expect(bus.registered).toContain(MODEL_EVENT);
  });
  // 等首次 model_status 落定
  await waitFor(() => {
    expect(utils.result.current.view.phase).not.toBe("loading");
  });
  return utils;
}

function emitProgress(payload: unknown) {
  act(() => {
    bus.emit(MODEL_EVENT, payload);
  });
}

let confirmMock: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  bus.handlers.clear();
  bus.registered.length = 0;
  backend.calls.length = 0;
  backend.state.handler = () => Promise.resolve({ phase: "unprepared" });
  confirmMock = vi.spyOn(window, "confirm").mockReturnValue(true);
});

describe("useModelManager —— 首次状态读取与归一化", () => {
  it("ready 状态带出 modelDir，phase 归一为 ready", async () => {
    backend.state.handler = () =>
      Promise.resolve({ phase: "ready", modelDir: "C:\\models\\paraformer" });
    const { result } = await mount();
    expect(result.current.view.phase).toBe("ready");
    expect(result.current.view.modelDir).toBe("C:\\models\\paraformer");
    expect(backend.calls).toContain("model_status");
  });

  it("下载中的状态把 completedBytes 同时当作 downloadedBytes", async () => {
    backend.state.handler = () =>
      Promise.resolve({
        phase: "downloading",
        step: "vad",
        resource: "vad.onnx",
        completedBytes: 1024,
        totalBytes: 2048,
      });
    const { result } = await mount();
    expect(result.current.view).toMatchObject({
      phase: "downloading",
      step: "vad",
      resource: "vad.onnx",
      completedBytes: 1024,
      downloadedBytes: 1024,
      totalBytes: 2048,
    });
  });

  it("未知 phase（如错误块）回落成 unprepared，允许用户下载", async () => {
    backend.state.handler = () => Promise.resolve({ phase: "谁都不认识", totalBytes: 9 });
    const { result } = await mount();
    expect(result.current.view.phase).toBe("unprepared");
    expect(result.current.view.totalBytes).toBe(9);
    expect(result.current.view.completedBytes).toBe(0);
  });

  it("null 返回值也当成未就绪，用默认总字节", async () => {
    backend.state.handler = () => Promise.resolve(null);
    const { result } = await mount();
    expect(result.current.view.phase).toBe("unprepared");
    expect(result.current.view.totalBytes).toBe(DEFAULT_TOTAL_BYTES);
  });

  it("读取失败时进入 error 并带上「读取模型状态失败：」前缀", async () => {
    backend.state.handler = () => Promise.reject("命令不存在");
    const { result } = await mount();
    expect(result.current.view.phase).toBe("error");
    expect(result.current.view.error).toBe("读取模型状态失败：命令不存在");
  });

  it("进度事件把视图切到 downloading 并更新字节数/来源", async () => {
    const { result } = await mount();
    emitProgress({
      step: "model",
      resource: "model.int8.onnx",
      completedBytes: 4096,
      totalBytes: 8192,
      source: "https://hf-mirror.com",
    });
    expect(result.current.view).toMatchObject({
      phase: "downloading",
      step: "model",
      resource: "model.int8.onnx",
      completedBytes: 4096,
      downloadedBytes: 4096,
      totalBytes: 8192,
      source: "https://hf-mirror.com",
      error: "",
    });
  });

  it("进度事件缺字段时保留旧值，非 record 的 payload 忽略", async () => {
    const { result } = await mount();
    emitProgress({ completedBytes: 100, resource: "x.onnx" });
    emitProgress({ completedBytes: 200 });
    expect(result.current.view.resource).toBe("x.onnx");
    expect(result.current.view.completedBytes).toBe(200);

    emitProgress("bad-payload");
    expect(result.current.view.resource).toBe("x.onnx");
    expect(result.current.view.completedBytes).toBe(200);
  });
});

describe("useModelManager —— download 状态机", () => {
  it("点击后立刻进 downloading，busy=true，命令返回后转 ready 并给成功提示", async () => {
    const gate = deferred<boolean>();
    let prepared = false;
    backend.state.handler = (command) => {
      if (command === "prepare_model") return gate.promise;
      return Promise.resolve(
        prepared ? { phase: "ready", modelDir: "D:\\m" } : { phase: "unprepared" },
      );
    };
    const { result } = await mount();
    expect(result.current.view.phase).toBe("unprepared");

    act(() => {
      void result.current.download();
    });
    // 不等命令返回：本地先切下载中，按钮不能还能点
    expect(result.current.view.phase).toBe("downloading");
    expect(result.current.view.step).toBe("model");
    expect(result.current.busy).toBe(true);

    prepared = true;
    await act(async () => {
      gate.resolve(true);
    });
    await waitFor(() => {
      expect(result.current.busy).toBe(false);
    });
    expect(result.current.view.phase).toBe("ready");
    expect(result.current.notice).toBe("模型文件检查完成，可以开始面试。");
  });

  it("ready 态复用按钮做校验：step 变 verifying 而不是 model", async () => {
    backend.state.handler = (command) => {
      if (command === "prepare_model") return new Promise(() => {});
      return Promise.resolve({ phase: "ready", modelDir: "D:\\m" });
    };
    const { result } = await mount();
    expect(result.current.view.phase).toBe("ready");

    act(() => {
      void result.current.download();
    });
    expect(result.current.view.step).toBe("verifying");
    expect(result.current.view.phase).toBe("downloading");
  });

  it("后端返回 false 表示用户取消：给取消提示并回读状态", async () => {
    let statusCalls = 0;
    backend.state.handler = (command) => {
      if (command === "prepare_model") return Promise.resolve(false);
      statusCalls += 1;
      return Promise.resolve({ phase: "unprepared", downloadedBytes: 512 });
    };
    const { result } = await mount();
    const before = statusCalls;

    await act(async () => {
      await result.current.download();
    });
    expect(result.current.notice).toBe(
      "已取消语音模型下载，已下载部分会保留，可稍后继续。",
    );
    expect(statusCalls).toBe(before + 1);
    expect(result.current.view.phase).toBe("unprepared");
    expect(result.current.view.downloadedBytes).toBe(512);
  });

  it("失败时剥掉后端「模型下载失败：」前缀，避免双重前缀", async () => {
    backend.state.handler = (command) => {
      if (command === "prepare_model") return Promise.reject("模型下载失败：网络不可达");
      return Promise.resolve({ phase: "unprepared" });
    };
    const { result } = await mount();

    await act(async () => {
      await result.current.download();
    });
    expect(result.current.notice).toBe("模型下载未完成：网络不可达");
    expect(result.current.view.error).toBe("模型下载失败：网络不可达");
    expect(result.current.view.phase).toBe("unprepared");
    expect(result.current.busy).toBe(false);
  });

  it("失败时若回读结果已是 ready，则保持 ready 且不留错误", async () => {
    backend.state.handler = (command) => {
      if (command === "prepare_model") return Promise.reject("无关错误");
      return Promise.resolve({ phase: "ready", modelDir: "E:\\m" });
    };
    const { result } = await mount();

    await act(async () => {
      await result.current.download();
    });
    expect(result.current.view.phase).toBe("ready");
    expect(result.current.view.error).toBe("");
    expect(result.current.notice).toBe("模型下载未完成：无关错误");
  });

  it("回读失败且当前是 downloading 时保留 downloading（不被错误覆盖）", async () => {
    backend.state.handler = (command) => {
      if (command === "prepare_model") return Promise.reject("磁盘满");
      return Promise.reject("状态接口挂了");
    };
    const { result } = await mount();
    await waitFor(() => {
      expect(result.current.view.phase).toBe("error");
    });

    await act(async () => {
      await result.current.download();
    });
    // download 先置 downloading，refresh 失败按「downloading 保留」分支处理，
    // 最终 error 字段是命令错误而不是读取错误
    expect(result.current.view.error).toBe("模型下载失败：磁盘满");
    expect(result.current.notice).toBe("模型下载未完成：磁盘满");
  });
});

describe("useModelManager —— cancel", () => {
  it("取消成功提示稍后更新，并在 1500ms 后回读状态", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let statusCalls = 0;
    backend.state.handler = (command) => {
      if (command === "cancel_prepare") return Promise.resolve(true);
      statusCalls += 1;
      return Promise.resolve({ phase: "downloading", completedBytes: 1 });
    };
    const { result, unmount } = renderHook(() => useModelManager());
    await act(async () => {
      await Promise.resolve();
    });
    const before = statusCalls;

    await act(async () => {
      await result.current.cancel();
    });
    expect(result.current.notice).toBe("已请求取消下载，稍后状态会自动更新。");
    expect(statusCalls).toBe(before);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(statusCalls).toBe(before + 1);
    unmount();
    vi.useRealTimers();
  });

  it("后端返回 false 表示当前没有下载在进行", async () => {
    backend.state.handler = (command) => {
      if (command === "cancel_prepare") return Promise.resolve(false);
      return Promise.resolve({ phase: "unprepared" });
    };
    const { result } = await mount();
    await act(async () => {
      await result.current.cancel();
    });
    expect(result.current.notice).toBe("当前没有正在进行的下载。");
  });

  it("取消失败给「取消下载失败：」提示", async () => {
    backend.state.handler = (command) => {
      if (command === "cancel_prepare") return Promise.reject("没有下载线程");
      return Promise.resolve({ phase: "downloading" });
    };
    const { result } = await mount();
    await act(async () => {
      await result.current.cancel();
    });
    expect(result.current.notice).toBe("取消下载失败：没有下载线程");
  });
});

describe("useModelManager —— remove", () => {
  it("取消确认时不发命令", async () => {
    confirmMock.mockReturnValue(false);
    const { result } = await mount();
    await act(async () => {
      await result.current.remove();
    });
    expect(backend.calls).not.toContain("remove_model");
    expect(result.current.notice).toBe("");
  });

  it("确认后删除并回读成未就绪", async () => {
    let removed = false;
    backend.state.handler = (command) => {
      if (command === "remove_model") {
        removed = true;
        return Promise.resolve(null);
      }
      return Promise.resolve({ phase: removed ? "unprepared" : "ready", modelDir: "D:\\m" });
    };
    const { result } = await mount();
    expect(result.current.view.phase).toBe("ready");

    await act(async () => {
      await result.current.remove();
    });
    expect(String(confirmMock.mock.calls[0][0])).toContain("确定删除语音模型吗？");
    expect(result.current.notice).toBe("语音模型已删除。");
    expect(result.current.view.phase).toBe("unprepared");
    expect(result.current.busy).toBe(false);
  });

  it("删除失败给「删除模型失败：」提示", async () => {
    backend.state.handler = (command) => {
      if (command === "remove_model") return Promise.reject("文件被占用");
      return Promise.resolve({ phase: "ready", modelDir: "D:\\m" });
    };
    const { result } = await mount();
    await act(async () => {
      await result.current.remove();
    });
    expect(result.current.notice).toBe("删除模型失败：文件被占用");
    expect(result.current.busy).toBe(false);
  });
});

describe("useModelManager —— 并发保护与清理", () => {
  it("refresh 正在飞行时不重复发 model_status", async () => {
    const gate = deferred<unknown>();
    let calls = 0;
    backend.state.handler = (command) => {
      if (command === "model_status") {
        calls += 1;
        return calls === 1 ? gate.promise : Promise.resolve({ phase: "unprepared" });
      }
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useModelManager());
    // 首次 refresh 挂着，后面的 refresh 应该直接 return
    act(() => {
      void result.current.refresh();
    });
    act(() => {
      void result.current.refresh();
    });
    // safeInvoke 先 await「IPC 是否就绪」，首次命令比同步断言晚一个 microtask；
    // 等它真正发出后再断言——仍然要求「两次 refresh 只发一次 model_status」。
    await waitFor(() => {
      expect(calls).toBe(1);
    });

    await act(async () => {
      gate.resolve({ phase: "unprepared" });
    });
    await waitFor(() => {
      expect(result.current.view.phase).toBe("unprepared");
    });

    // 第一次结束后又能正常刷新
    await act(async () => {
      await result.current.refresh();
    });
    expect(calls).toBe(2);
  });

  it("卸载后不再把状态写回（进度事件也不会泄漏）", async () => {
    const gate = deferred<unknown>();
    backend.state.handler = (command) => {
      if (command === "model_status") return gate.promise;
      return Promise.resolve(null);
    };
    const { result, unmount } = renderHook(() => useModelManager());
    const snapshot = result.current.view;
    unmount();
    await act(async () => {
      gate.resolve({ phase: "ready", modelDir: "X" });
    });
    expect(result.current.view).toEqual(snapshot);
    expect(bus.handlers.get(MODEL_EVENT)?.length ?? 0).toBe(0);
  });
});