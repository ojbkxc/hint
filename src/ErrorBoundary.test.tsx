import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ErrorBoundary from "./ErrorBoundary";

const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  return { calls };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    return Promise.resolve(null);
  }),
}));

/** 渲染期必抛的组件。 */
function Boom(): never {
  throw new Error("Failed to execute 'insertBefore' on 'Node'");
}

beforeEach(() => {
  backend.calls.length = 0;
  // React 会把错误打到 console.error，测试里静音以免刷屏
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ErrorBoundary —— 渲染异常兜底", () => {
  it("子组件抛错时不显示空白，而是给出提示与自救按钮", () => {
    // 实测过一次真实崩溃：外部脚本改了 React 管理的节点，破坏虚拟 DOM 假设，
    // 整棵树被卸载 → 浮层变成一片空白，用户看起来像「程序死了」。
    render(
      <ErrorBoundary surface="overlay">
        <Boom />
      </ErrorBoundary>,
    );

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText("浮层渲染出错")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新加载" })).toBeInTheDocument();
  });

  it("浮层额外给「隐藏浮层」出口（面试中卡住要能让开视线）", async () => {
    const user = userEvent.setup();
    render(
      <ErrorBoundary surface="overlay">
        <Boom />
      </ErrorBoundary>,
    );

    await user.click(screen.getByRole("button", { name: "隐藏浮层" }));
    // 按钮走的是 `await import("./lib/tauri")` + safeInvoke 的异步链：
    // click 返回时动态 import 还没结算完，直接断言会读到空的 calls。
    // 这里只等异步链落地，判据（必须调用 hide_overlay）没有放宽。
    await waitFor(() =>
      expect(backend.calls.some((c) => c.command === "hide_overlay")).toBe(true),
    );
  });

  it("设置页与选区窗口不给隐藏浮层按钮（不是它们的职责）", () => {
    const { unmount } = render(
      <ErrorBoundary surface="dashboard">
        <Boom />
      </ErrorBoundary>,
    );
    expect(screen.getByText("设置页渲染出错")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "隐藏浮层" })).toBeNull();
    unmount();

    render(
      <ErrorBoundary surface="capture">
        <Boom />
      </ErrorBoundary>,
    );
    expect(screen.getByText("选区窗口渲染出错")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "隐藏浮层" })).toBeNull();
  });

  it("错误信息截断显示，不整段铺开", () => {
    const long = "x".repeat(500);
    function LongBoom(): never {
      throw new Error(long);
    }
    render(
      <ErrorBoundary surface="overlay">
        <LongBoom />
      </ErrorBoundary>,
    );
    const detail = document.querySelector(".crash-detail");
    expect(detail?.textContent?.length).toBeLessThanOrEqual(160);
  });

  it("正常子组件原样渲染（兜底不干扰常规路径）", () => {
    render(
      <ErrorBoundary surface="overlay">
        <p>正常内容</p>
      </ErrorBoundary>,
    );
    expect(screen.getByText("正常内容")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
