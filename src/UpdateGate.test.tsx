import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const state: {
    latest: unknown;
    downloadResult: unknown;
    installResult: unknown;
  } = { latest: null, downloadResult: "/tmp/setup.exe", installResult: null };
  return { calls, state };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    if (command === "check_update") {
      return typeof backend.state.latest === "function"
        ? (backend.state.latest as () => unknown)()
        : Promise.resolve(backend.state.latest);
    }
    if (command === "download_update") {
      return typeof backend.state.downloadResult === "function"
        ? (backend.state.downloadResult as () => unknown)()
        : Promise.resolve(backend.state.downloadResult);
    }
    if (command === "install_update") {
      return typeof backend.state.installResult === "function"
        ? (backend.state.installResult as () => unknown)()
        : Promise.resolve(backend.state.installResult);
    }
    return Promise.resolve(null);
  }),
}));

import UpdateGate from "./UpdateGate";

const UPDATE = {
  version: "1.1.99",
  installerUrl: "https://example.com/x.exe",
  installerName: "hint_1.1.99_x64-setup.exe",
  notes: "## 更新",
};

beforeEach(() => {
  backend.calls.length = 0;
  backend.state.latest = null;
  backend.state.downloadResult = "/tmp/setup.exe";
  backend.state.installResult = null;
});

function commandsCalled() {
  return backend.calls.map((call) => call.command);
}

describe("UpdateGate —— 启动检查更新", () => {
  it("没有新版本时直接放行（用户看不到遮罩）", async () => {
    const onDone = vi.fn();
    render(<UpdateGate onDone={onDone} />);

    await waitFor(() => expect(onDone).toHaveBeenCalled());
    // 没有新版本就不该发起下载。
    expect(commandsCalled()).not.toContain("download_update");
  });

  it("检查更新失败时也放行——网络不通不能让用户打不开软件", async () => {
    // 命令层把所有失败折成 Ok(None)；这里模拟 IPC 层直接抛错。
    backend.state.latest = () => Promise.reject("network unreachable");
    const onDone = vi.fn();
    render(<UpdateGate onDone={onDone} />);

    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(screen.queryByText(/更新失败/)).toBeNull();
  });

  it("发现新版本时自动下载并安装，全程不放行", async () => {
    // 下载故意慢一拍：否则界面会瞬间跳到「正在安装」，测不到 updating 阶段。
    backend.state.latest = UPDATE;
    backend.state.downloadResult = () =>
      new Promise((resolve) => setTimeout(() => resolve("/tmp/setup.exe"), 50));
    const onDone = vi.fn();
    render(<UpdateGate onDone={onDone} />);

    await waitFor(() => expect(screen.getByText(/正在更新 hint/)).toBeInTheDocument());
    // 版本号在 <strong> 里，且 <p> 里混了「说明文字 + 子元素」——
    // getByText 对这种混合结构匹配不稳，直接查 DOM。
    expect(document.querySelector(".update-version")?.textContent).toContain("1.1.99");

    await waitFor(() => expect(commandsCalled()).toContain("download_update"));
    await waitFor(() => expect(commandsCalled()).toContain("install_update"));

    // 关键：下载拿到的路径要原样交给安装命令。
    const installCall = backend.calls.find((c) => c.command === "install_update");
    expect(installCall?.args).toEqual({ path: "/tmp/setup.exe" });

    // 全程不允许放行：强制更新的意义就在这里。
    expect(onDone).not.toHaveBeenCalled();
  });

  it("下载失败时显示原因，且提供重试与跳过", async () => {
    backend.state.latest = UPDATE;
    backend.state.downloadResult = () => Promise.reject("下载中断：连接重置");
    const onDone = vi.fn();
    render(<UpdateGate onDone={onDone} />);

    await waitFor(() => expect(screen.getByText(/更新失败/)).toBeInTheDocument());
    expect(screen.getByText(/下载中断/)).toBeInTheDocument();
    expect(screen.getByText("重试")).toBeInTheDocument();
    expect(screen.getByText("跳过本次")).toBeInTheDocument();
    // 失败后不再自动往下走。
    expect(commandsCalled()).not.toContain("install_update");
    expect(onDone).not.toHaveBeenCalled();
  });

  it("点「跳过本次」放行进主界面", async () => {
    backend.state.latest = UPDATE;
    backend.state.downloadResult = () => Promise.reject("boom");
    const onDone = vi.fn();
    render(<UpdateGate onDone={onDone} />);

    await waitFor(() => expect(screen.getByText("跳过本次")).toBeInTheDocument());
    await userEvent.click(screen.getByText("跳过本次"));
    expect(onDone).toHaveBeenCalled();
  });

  it("点「重试」重新走一遍检查", async () => {
    backend.state.latest = UPDATE;
    backend.state.downloadResult = () => Promise.reject("boom");
    const onDone = vi.fn();
    render(<UpdateGate onDone={onDone} />);

    await waitFor(() => expect(screen.getByText("重试")).toBeInTheDocument());
    const before = commandsCalled().filter((c) => c === "check_update").length;
    await userEvent.click(screen.getByText("重试"));
    await waitFor(() => {
      const after = commandsCalled().filter((c) => c === "check_update").length;
      expect(after).toBeGreaterThan(before);
    });
    expect(onDone).not.toHaveBeenCalled();
  });

  it("安装失败时显示原因而不是假装成功", async () => {
    backend.state.latest = UPDATE;
    backend.state.installResult = () => Promise.reject("安装包不存在：/tmp/x.exe");
    render(<UpdateGate onDone={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/安装失败/)).toBeInTheDocument());
    expect(screen.getByText(/安装包不存在/)).toBeInTheDocument();
  });
});
