import { act, cleanup, screen } from "@testing-library/react";
import { StrictMode } from "react";
import type { ReactElement, ReactNode } from "react";
import type { Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * main.tsx 在顶层就把 `<StrictMode><Root /></StrictMode>` 交给 createRoot，
 * 没有导出任何东西——所以测试只能替换 `react-dom/client.createRoot`
 * 截住真正挂载的那棵树，再让真实 React 把它渲染进 jsdom。
 *
 * 关键点：
 * - createRoot 必须**委托真实实现**（@testing-library/react 自己也用
 *   react-dom/client，全量假造会让两边都渲染不出东西），只在外面套一层计数。
 * - 每个用例 `vi.resetModules()` 后重新 import，才能让新的 location.search /
 *   getCurrentWindow 行为生效（label 在模块顶层求值一次，缓存住就换不掉）。
 */
const probe = vi.hoisted(() => ({
  createRootCalls: 0,
  renderCalls: 0,
  roots: [] as Root[],
  elements: [] as ReactNode[],
  overlayCalls: 0,
  captureCalls: 0,
  dashboardCalls: 0,
  windowCalls: 0,
  /** getCurrentWindow 的行为：返回固定 label 或直接抛错（纯浏览器预览）。 */
  windowMode: "label" as "label" | "throw",
  windowLabel: "main",
}));

vi.mock("react-dom/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-dom/client")>();
  // 实测形状：命名导出 createRoot / hydrateRoot + 一个 default 对象（同名字段）。
  // main.tsx 走的是 `import ReactDOM from "react-dom/client"`，即 default 那侧；
  // @testing-library/react 走命名导出。两侧都换成计数版，才拦得住 main 的顶层挂载。
  const actualDefault = (actual as unknown as { default: typeof actual }).default ?? actual;
  const createRootImpl = actual.createRoot;

  function countingCreateRoot(...args: Parameters<typeof createRootImpl>): Root {
    probe.createRootCalls += 1;
    const root = createRootImpl(...args);
    probe.roots.push(root);
    // main.tsx 只用到 render；unmount 留给 afterEach 调用真实 root。
    return {
      render: (node: ReactNode) => {
        probe.renderCalls += 1;
        probe.elements.push(node);
        root.render(node);
      },
      unmount: () => root.unmount(),
    } as unknown as Root;
  }

  return {
    ...actual,
    createRoot: countingCreateRoot,
    default: { ...actualDefault, createRoot: countingCreateRoot },
  };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => {
    probe.windowCalls += 1;
    if (probe.windowMode === "throw") throw new Error("window.__TAURI_INTERNALS__ is undefined");
    return { label: probe.windowLabel };
  },
}));

// 三个根组件只保留「有没有被渲染」这一件事，断言打在分发结果上而不是组件内部。
vi.mock("./Overlay", async () => {
  const React = await import("react");
  return {
    default: function OverlayMock() {
      probe.overlayCalls += 1;
      return React.createElement("div", { "data-testid": "overlay" });
    },
  };
});

vi.mock("./CaptureOverlay", async () => {
  const React = await import("react");
  return {
    default: function CaptureOverlayMock() {
      probe.captureCalls += 1;
      return React.createElement("div", { "data-testid": "capture" });
    },
  };
});

vi.mock("./Dashboard", async () => {
  const React = await import("react");
  return {
    default: function DashboardMock() {
      probe.dashboardCalls += 1;
      return React.createElement("div", { "data-testid": "dashboard" });
    },
  };
});

function resetProbe(): void {
  probe.createRootCalls = 0;
  probe.renderCalls = 0;
  probe.elements.length = 0;
  probe.overlayCalls = 0;
  probe.captureCalls = 0;
  probe.dashboardCalls = 0;
  probe.windowCalls = 0;
  probe.windowMode = "label";
  probe.windowLabel = "main";
}

/** 改 location.search（replaceState 不触发 jsdom 的导航）。 */
function setSearch(search: string): void {
  window.history.replaceState({}, "", `/${search}`);
}

/** 重新执行一次 main.tsx 的顶层副作用，等 StrictMode 下的挂载落定。 */
async function bootMain(): Promise<void> {
  await act(async () => {
    await import("./main");
  });
}

beforeEach(() => {
  vi.resetModules();
  resetProbe();
  document.body.innerHTML = '<div id="root"></div>';
  setSearch("");
});

afterEach(() => {
  act(() => {
    for (const root of probe.roots.splice(0)) root.unmount();
  });
  cleanup();
  document.body.innerHTML = "";
});

describe("main 入口的窗口分发", () => {
  it("?window=overlay 直接渲染 Overlay，且不碰 Tauri 窗口 API", async () => {
    setSearch("?window=overlay");

    await bootMain();

    expect(screen.getByTestId("overlay")).toBeInTheDocument();
    expect(screen.queryByTestId("dashboard")).toBeNull();
    expect(screen.queryByTestId("capture")).toBeNull();
    // 查询参数优先：命中后不该再去读窗口 label。
    expect(probe.windowCalls).toBe(0);
  });

  it("?window=capture 渲染 CaptureOverlay", async () => {
    setSearch("?window=capture");

    await bootMain();

    expect(screen.getByTestId("capture")).toBeInTheDocument();
    expect(screen.queryByTestId("overlay")).toBeNull();
    expect(screen.queryByTestId("dashboard")).toBeNull();
    expect(probe.windowCalls).toBe(0);
  });

  it("无查询参数时读 Tauri 窗口 label，label=main 渲染 Dashboard", async () => {
    probe.windowLabel = "main";

    await bootMain();

    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
    expect(screen.queryByTestId("overlay")).toBeNull();
    expect(screen.queryByTestId("capture")).toBeNull();
    // 只有真的走了 label 分支才会调用 getCurrentWindow。
    expect(probe.windowCalls).toBe(1);
  });

  it("getCurrentWindow 抛错（纯浏览器预览）时回落 Dashboard", async () => {
    probe.windowMode = "throw";

    await bootMain();

    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
    expect(probe.windowCalls).toBe(1);
  });

  it("label 为未知值（settings）时回落 Dashboard", async () => {
    probe.windowLabel = "settings";

    await bootMain();

    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
    expect(screen.queryByTestId("overlay")).toBeNull();
    expect(screen.queryByTestId("capture")).toBeNull();
  });

  it("未知的 ?window=Overlay（大小写不符）不当作预览参数，回落 label 分支", async () => {
    setSearch("?window=Overlay");
    probe.windowLabel = "main";

    await bootMain();

    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
    expect(probe.windowCalls).toBe(1);
  });

  it("StrictMode 下只挂载一次：单个 createRoot、单次 render、DOM 单节点", async () => {
    setSearch("?window=overlay");

    await bootMain();

    // 顶层副作用只跑一次。
    expect(probe.createRootCalls).toBe(1);
    expect(probe.renderCalls).toBe(1);
    // 交给 createRoot 的树被 StrictMode 包住（React 19 开发构建会双调用函数体，
    // 但 DOM 只挂一份；若有人把 StrictMode 去掉或改成双重 createRoot，下面的
    // 计数会立刻变红）。
    const element = probe.elements[0] as ReactElement;
    expect(element.type).toBe(StrictMode);
    expect(probe.overlayCalls).toBe(2);
    expect(document.querySelectorAll('[data-testid="overlay"]')).toHaveLength(1);
    expect(document.querySelectorAll("#root > *")).toHaveLength(1);
    expect(screen.getAllByTestId("overlay")).toHaveLength(1);
  });
});
