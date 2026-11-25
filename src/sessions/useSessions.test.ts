import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type InvokeHandler = (command: string, args: unknown) => Promise<unknown>;

const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const state: { handler: InvokeHandler } = {
    handler: () => Promise.resolve(null),
  };
  return { calls, state };
});

const dialogState = vi.hoisted(() => ({
  // save() 的返回值：路径字符串 = 用户选了位置；null = 用户取消
  saveResult: "# 占位" as string | null,
  saveShouldThrow: false,
  saveCalls: [] as unknown[],
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn((opts: unknown) => {
    dialogState.saveCalls.push(opts);
    if (dialogState.saveShouldThrow) return Promise.reject(new Error("dialog broken"));
    return Promise.resolve(dialogState.saveResult);
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    return backend.state.handler(command, args);
  }),
}));

import { useSessions } from "./useSessions";

/** 手动控制 resolve 时机的 promise，用来制造「旧请求后返回」。 */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function commandsCalled(): string[] {
  return backend.calls.map((item) => item.command);
}

function lastArgsOf(command: string): unknown {
  const matches = backend.calls.filter((item) => item.command === command);
  return matches[matches.length - 1]?.args;
}

const sessionsPayload = (ids: string[]) =>
  ids.map((id) => ({
    id,
    startedAt: `2026-01-0${id} 10:00:00`,
    endedAt: "",
    model: `m-${id}`,
    note: `会话 ${id}`,
  }));

let confirmMock: ReturnType<typeof vi.spyOn>;
let createObjectURL: ReturnType<typeof vi.fn>;

beforeEach(() => {
  backend.calls.length = 0;
  backend.state.handler = () => Promise.resolve(null);
  confirmMock = vi.spyOn(window, "confirm").mockReturnValue(true);
  createObjectURL = vi.fn(() => "blob:hint");
  Object.defineProperty(URL, "createObjectURL", {
    configurable: true,
    writable: true,
    value: createObjectURL,
  });
  Object.defineProperty(URL, "revokeObjectURL", {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
});

afterEach(() => {
  confirmMock.mockRestore();
  vi.useRealTimers();
});

describe("useSessions —— 列表请求序号（旧响应不覆盖新列表）", () => {
  it("先发的请求后返回时不覆盖后发请求的结果", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    const queue = [first, second];
    backend.state.handler = (command) => {
      if (command === "list_sessions") return queue.shift()!.promise;
      return Promise.resolve(null);
    };

    const { result } = renderHook(() => useSessions(false));
    act(() => {
      void result.current.refresh();
    });
    act(() => {
      void result.current.refresh();
    });
    // safeInvoke 内部现在会先 await「IPC 是否就绪」，命令发出比同步断言晚
    // 一个 microtask；用 waitFor 等它发出来，断言强度不变（仍要求两次都发出）。
    await waitFor(() => {
      expect(commandsCalled().filter((c) => c === "list_sessions")).toHaveLength(2);
    });

    // 后发的先返回：新列表上屏
    await act(async () => {
      second.resolve(sessionsPayload(["2"]));
    });
    await waitFor(() => {
      expect(result.current.sessions.map((s) => s.id)).toEqual(["2"]);
    });
    expect(result.current.loading).toBe(false);

    // 旧响应姗姗来迟：不能把新列表顶掉
    await act(async () => {
      first.resolve(sessionsPayload(["1"]));
    });
    expect(result.current.sessions.map((s) => s.id)).toEqual(["2"]);
  });

  it("后发的请求失败时，旧成功响应同样不能覆盖错误态", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    const queue = [first, second];
    backend.state.handler = (command) => {
      if (command === "list_sessions") return queue.shift()!.promise;
      return Promise.resolve(null);
    };

    const { result } = renderHook(() => useSessions(false));
    act(() => {
      void result.current.refresh();
    });
    act(() => {
      void result.current.refresh();
    });

    await act(async () => {
      second.reject("磁盘不可读");
    });
    await waitFor(() => {
      expect(result.current.error).toBe("读取会话失败：磁盘不可读");
    });
    expect(result.current.sessions).toEqual([]);

    await act(async () => {
      first.resolve(sessionsPayload(["1"]));
    });
    expect(result.current.error).toBe("读取会话失败：磁盘不可读");
    expect(result.current.sessions).toEqual([]);
  });

  it("enabled 为 false 时不发请求，切到 true 后自动拉一次", async () => {
    backend.state.handler = () => Promise.resolve(sessionsPayload(["7"]));
    const { result, rerender } = renderHook(
      ({ enabled }: { enabled: boolean }) => useSessions(enabled),
      { initialProps: { enabled: false } },
    );
    expect(commandsCalled()).not.toContain("list_sessions");

    rerender({ enabled: true });
    await waitFor(() => {
      expect(result.current.sessions.map((s) => s.id)).toEqual(["7"]);
    });
    expect(commandsCalled().filter((c) => c === "list_sessions")).toHaveLength(1);
  });

  it("刷新成功后 loading 复位，失败时清空列表并带中文前缀", async () => {
    backend.state.handler = () => Promise.reject("连接中断");
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBe("读取会话失败：连接中断");
    expect(result.current.sessions).toEqual([]);
  });
});

describe("useSessions —— 详情请求序号", () => {
  it("快速连点两条会话时只接受最后一次的详情", async () => {
    const slow = deferred<unknown>();
    const fast = deferred<unknown>();
    const pending = new Map<string, ReturnType<typeof deferred<unknown>>>([
      ["a", slow],
      ["b", fast],
    ]);
    backend.state.handler = (command, args) => {
      if (command === "session_detail") {
        const id = (args as { id: string }).id;
        return pending.get(id)!.promise;
      }
      return Promise.resolve(null);
    };

    const { result } = renderHook(() => useSessions(false));
    act(() => {
      void result.current.open("a");
    });
    act(() => {
      void result.current.open("b");
    });
    expect(result.current.selectedId).toBe("b");
    expect(result.current.detailLoading).toBe(true);

    await act(async () => {
      fast.resolve({ id: "b", segments: [{ text: "B 的转录" }], answers: [] });
    });
    await waitFor(() => {
      expect(result.current.detail?.id).toBe("b");
    });

    // 慢的 a 后返回：不能把 b 的详情换掉
    await act(async () => {
      slow.resolve({ id: "a", segments: [{ text: "A 的转录" }], answers: [] });
    });
    expect(result.current.detail?.id).toBe("b");
    expect(result.current.detail?.segments[0].text).toBe("B 的转录");
    expect(result.current.detailLoading).toBe(false);
  });

  it("打开会话时先清空上一条的详情、错误与提示", async () => {
    const detail = deferred<unknown>();
    backend.state.handler = (command) => {
      if (command === "session_detail") return detail.promise;
      if (command === "export_session_markdown") return Promise.reject("导出失败：没有内容");
      return Promise.resolve(null);
    };

    const { result } = renderHook(() => useSessions(false));
    act(() => {
      // 不 await：detail promise 还挂着，用来模拟「上一条详情没回来」
      void result.current.open("x");
    });

    // 先制造一个 detailError，再打开另一条
    await act(async () => {
      detail.reject("读盘失败");
    });
    await waitFor(() => {
      expect(result.current.detailError).toBe("加载会话失败：读盘失败");
    });

    backend.state.handler = () => Promise.resolve({ id: "y", segments: [], answers: [] });
    await act(async () => {
      await result.current.open("y");
    });
    expect(result.current.detailError).toBe("");
    expect(result.current.notice).toBe("");
    expect(result.current.selectedId).toBe("y");
    expect(result.current.detail?.id).toBe("y");
  });

  it("详情请求失败时给出带前缀的错误且 detail 保持 null", async () => {
    backend.state.handler = () => Promise.reject("会话被删除");
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.open("ghost");
    });
    expect(result.current.detailError).toBe("加载会话失败：会话被删除");
    expect(result.current.detail).toBeNull();
    expect(result.current.detailLoading).toBe(false);
  });
});

describe("useSessions —— 导出 Markdown", () => {
  beforeEach(() => {
    dialogState.saveResult = "# 转录\n内容";
    dialogState.saveShouldThrow = false;
    dialogState.saveCalls.length = 0;
  });

  it("先弹保存对话框，再把选中的路径交给后端写盘", async () => {
    // 原实现用浏览器 Blob 下载，真机验证过 WebView2 里文件根本落不下来
    // （下载事件走完 completed，但用户目录/AppData/Temp 全找不到），
    // 所以改成「选路径 → 后端写盘」。
    backend.state.handler = (command) => {
      if (command === "export_session_file") return Promise.resolve(null);
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.open("2026/01/01 10:00");
    });
    await act(async () => {
      await result.current.exportMarkdown();
    });

    expect(dialogState.saveCalls).toHaveLength(1);
    expect(lastArgsOf("export_session_file")).toEqual({
      id: "2026/01/01 10:00",
      path: "# 转录\n内容",
    });
    expect(result.current.notice).toBe("已导出 Markdown 文件");
  });

  it("用户在对话框里取消：不发写盘命令、不提示成功", async () => {
    dialogState.saveResult = null;
    backend.state.handler = () => Promise.resolve(null);
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.open("s1");
    });
    await act(async () => {
      await result.current.exportMarkdown();
    });

    expect(dialogState.saveCalls).toHaveLength(1);
    expect(commandsCalled()).not.toContain("export_session_file");
    expect(result.current.notice).toBe("");
  });

  it("写盘失败时把原因显示出来（不谎报成功）", async () => {
    backend.state.handler = (command) => {
      if (command === "export_session_file") return Promise.reject("导出失败：磁盘只读");
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.open("s1");
    });
    await act(async () => {
      await result.current.exportMarkdown();
    });

    expect(result.current.notice).toBe("导出失败：磁盘只读");
    expect(result.current.notice).not.toContain("导出失败：导出失败：");
  });

  it("后端错误不带前缀时补上「导出失败：」", async () => {
    backend.state.handler = (command) => {
      if (command === "export_session_file") return Promise.reject("io error");
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.open("s1");
    });
    await act(async () => {
      await result.current.exportMarkdown();
    });

    expect(result.current.notice).toBe("导出失败：io error");
  });

  it("对话框本身报错时也给出可读提示", async () => {
    dialogState.saveShouldThrow = true;
    backend.state.handler = () => Promise.resolve(null);
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.open("s1");
    });
    await act(async () => {
      await result.current.exportMarkdown();
    });

    expect(result.current.notice).toContain("导出失败");
    expect(result.current.notice).toContain("保存对话框");
    expect(commandsCalled()).not.toContain("export_session_file");
  });

  it("没有选中会话时导出直接返回，既不弹框也不发命令", async () => {
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.exportMarkdown();
    });
    expect(dialogState.saveCalls).toHaveLength(0);
    expect(commandsCalled()).not.toContain("export_session_file");
    expect(result.current.notice).toBe("");
  });
});

describe("useSessions —— 删除单条", () => {
  it("确认后删除，剥掉后端「删除会话失败：」前缀并刷新列表", async () => {
    backend.state.handler = (command) => {
      if (command === "list_sessions") return Promise.resolve(sessionsPayload(["1"]));
      if (command === "delete_session") return Promise.reject("删除会话失败：文件被占用");
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    await act(async () => {
      await result.current.open("1");
    });

    await act(async () => {
      await result.current.remove();
    });
    expect(confirmMock).toHaveBeenCalledTimes(1);
    expect(String(confirmMock.mock.calls[0][0])).toContain("「2026-01-01 10:00:00」");
    expect(result.current.notice).toBe("删除失败：文件被占用");
  });

  it("取消确认时不发删除命令", async () => {
    confirmMock.mockReturnValue(false);
    backend.state.handler = (command) => {
      if (command === "list_sessions") return Promise.resolve(sessionsPayload(["1"]));
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    await act(async () => {
      await result.current.open("1");
    });
    await act(async () => {
      await result.current.remove();
    });
    expect(commandsCalled()).not.toContain("delete_session");
    expect(result.current.selectedId).toBe("1");
  });

  it("删除成功后清空选中与详情并重新拉列表", async () => {
    let listCall = 0;
    backend.state.handler = (command) => {
      if (command === "list_sessions") {
        listCall += 1;
        return Promise.resolve(sessionsPayload(listCall === 1 ? ["1"] : []));
      }
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    await act(async () => {
      await result.current.open("1");
    });
    await act(async () => {
      await result.current.remove();
    });

    expect(result.current.notice).toBe("会话已删除");
    expect(result.current.selectedId).toBe("");
    expect(result.current.detail).toBeNull();
    // safeInvoke 内部现在会先 await「IPC 是否就绪」，命令发出比同步断言晚
    // 一个 microtask；用 waitFor 等它发出来，断言强度不变（仍要求两次都发出）。
    await waitFor(() => {
      expect(commandsCalled().filter((c) => c === "list_sessions")).toHaveLength(2);
    });
    expect(result.current.sessions).toEqual([]);
  });

  it("没有选中会话时删除直接返回（不弹确认）", async () => {
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.remove();
    });
    expect(confirmMock).not.toHaveBeenCalled();
    expect(commandsCalled()).not.toContain("delete_session");
  });
});

describe("useSessions —— 删除全部", () => {
  it("两次确认后才删除，成功提示带条数", async () => {
    let listCall = 0;
    backend.state.handler = (command) => {
      if (command === "list_sessions") {
        listCall += 1;
        return Promise.resolve(sessionsPayload(listCall === 1 ? ["1", "2"] : []));
      }
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    await act(async () => {
      await result.current.removeAll();
    });

    expect(confirmMock).toHaveBeenCalledTimes(2);
    expect(String(confirmMock.mock.calls[0][0])).toContain("全部 2 条");
    expect(String(confirmMock.mock.calls[1][0])).toContain("再次确认");
    expect(commandsCalled()).toContain("delete_all_sessions");
    expect(result.current.notice).toBe("全部 2 条会话记录已清空");
    expect(result.current.sessions).toEqual([]);
  });

  it("第二次确认取消时不发删除命令", async () => {
    confirmMock.mockReturnValueOnce(true).mockReturnValueOnce(false);
    backend.state.handler = () =>
      Promise.resolve(sessionsPayload(["1", "2"]));
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    await act(async () => {
      await result.current.removeAll();
    });
    expect(commandsCalled()).not.toContain("delete_all_sessions");
    expect(result.current.notice).toBe("");
  });

  it("列表为空时清空全部直接返回，不弹确认", async () => {
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.removeAll();
    });
    expect(confirmMock).not.toHaveBeenCalled();
    expect(commandsCalled()).not.toContain("delete_all_sessions");
  });

  it("清空失败时保留原列表并提示后端错误", async () => {
    backend.state.handler = (command) => {
      if (command === "list_sessions") return Promise.resolve(sessionsPayload(["1"]));
      if (command === "delete_all_sessions") return Promise.reject("权限不足");
      return Promise.resolve(null);
    };
    const { result } = renderHook(() => useSessions(false));
    await act(async () => {
      await result.current.refresh();
    });
    await act(async () => {
      await result.current.removeAll();
    });
    expect(result.current.notice).toBe("清空失败：权限不足");
    expect(result.current.sessions.map((s) => s.id)).toEqual(["1"]);
  });
});