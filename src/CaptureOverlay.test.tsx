import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

type BackendCall = { command: string; args: unknown };

const backend = vi.hoisted(() => {
  const calls: BackendCall[] = [];
  const routes = new Map<string, (args: unknown) => unknown>();
  return { calls, routes };
});

// safeInvoke 只关心 @tauri-apps/api/core 的 invoke：抛错代表后端失败，
// 返回原始数据代表成功（这里的数据对 finish_capture / cancel_capture 都是 null）。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    const handler = backend.routes.get(command);
    return Promise.resolve().then(() => (handler ? handler(args) : null));
  }),
}));

import CaptureOverlay from "./CaptureOverlay";

function callsTo(command: string): BackendCall[] {
  return backend.calls.filter((call) => call.command === command);
}

function route(command: string, handler: (args: unknown) => unknown) {
  backend.routes.set(command, handler);
}

/**
 * jsdom 没有实现 Pointer Events 的指针捕获（Element.prototype 上没有
 * setPointerCapture / hasPointerCapture / releasePointerCapture），组件会直接抛错。
 * 这里在根节点上按规范语义补一份可断言的实现：捕获集合 + 释放即移除。
 */
function installPointerCapture(root: HTMLElement) {
  const captured = new Set<number>();
  const setCapture = vi.fn((pointerId: number) => {
    captured.add(pointerId);
  });
  const hasCapture = vi.fn((pointerId: number) => captured.has(pointerId));
  const releaseCapture = vi.fn((pointerId: number) => {
    captured.delete(pointerId);
  });
  root.setPointerCapture = setCapture;
  root.hasPointerCapture = hasCapture;
  root.releasePointerCapture = releaseCapture;
  return { captured, setCapture, hasCapture, releaseCapture };
}

function mount() {
  const utils = render(<CaptureOverlay />);
  const root = utils.container.querySelector(".capture-root") as HTMLElement;
  const capture = installPointerCapture(root);
  return { ...utils, root, capture };
}

function rectBox() {
  return document.querySelector(".capture-rect") as HTMLElement | null;
}

function drag(
  root: HTMLElement,
  from: { x: number; y: number },
  to: { x: number; y: number },
) {
  fireEvent.pointerDown(root, {
    button: 0,
    pointerId: 1,
    clientX: from.x,
    clientY: from.y,
  });
  fireEvent.pointerMove(root, { pointerId: 1, clientX: to.x, clientY: to.y });
  fireEvent.pointerUp(root, { pointerId: 1, clientX: to.x, clientY: to.y });
}

beforeEach(() => {
  backend.calls.length = 0;
  backend.routes.clear();
});

describe("CaptureOverlay —— 初始态", () => {
  it("初始不渲染选区框，提示文案含拖拽与 Esc 两条信息", () => {
    const { container } = mount();
    expect(rectBox()).toBeNull();
    expect(screen.getByText("拖拽框选要提问的区域")).toBeInTheDocument();
    expect(
      screen.getByText("松开鼠标后截取；Esc 或右键取消。"),
    ).toBeInTheDocument();
    expect(container.querySelector(".capture-error")).toBeNull();
  });

  it("按下指针即进入选区态：起点与终点相同，尺寸 0 × 0", () => {
    const { root, capture } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 40, clientY: 50 });

    expect(capture.setCapture).toHaveBeenCalledWith(1);
    const box = rectBox() as HTMLElement;
    expect(box).not.toBeNull();
    expect(box.style.left).toBe("40px");
    expect(box.style.top).toBe("50px");
    expect(box.style.width).toBe("0px");
    expect(box.style.height).toBe("0px");
    expect(box.textContent).toBe("0 × 0");
  });

  it("非左键按下不开始选区", () => {
    const { root, capture } = mount();
    fireEvent.pointerDown(root, { button: 2, pointerId: 1, clientX: 40, clientY: 50 });

    expect(capture.setCapture).not.toHaveBeenCalled();
    expect(rectBox()).toBeNull();
  });

  it("没有按下就移动/抬起不产生选区，也不调后端", async () => {
    const { root } = mount();
    fireEvent.pointerMove(root, { pointerId: 1, clientX: 90, clientY: 90 });
    fireEvent.pointerUp(root, { pointerId: 1, clientX: 90, clientY: 90 });

    expect(rectBox()).toBeNull();
    expect(callsTo("finish_capture")).toHaveLength(0);
    expect(screen.queryByText("选区太小，请重新框选")).not.toBeInTheDocument();
  });
});

describe("CaptureOverlay —— normalizeRect 四方向", () => {
  it.each([
    ["左上→右下", { x: 120, y: 130 }, { x: 220, y: 210 }],
    ["右下→左上（反向）", { x: 220, y: 210 }, { x: 120, y: 130 }],
    ["左下→右上", { x: 120, y: 210 }, { x: 220, y: 130 }],
    ["右上→左下", { x: 220, y: 130 }, { x: 120, y: 210 }],
  ])("%s 都归一成 left/top 取小、宽高取绝对值", (_label, from, to) => {
    const { root } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: from.x, clientY: from.y });
    fireEvent.pointerMove(root, { pointerId: 1, clientX: to.x, clientY: to.y });

    const box = rectBox() as HTMLElement;
    expect(box.style.left).toBe("120px");
    expect(box.style.top).toBe("130px");
    expect(box.style.width).toBe("100px");
    expect(box.style.height).toBe("80px");
    expect(box.textContent).toBe("100 × 80");
  });

  it("拖拽过程中尺寸文本实时跟随指针", () => {
    const { root } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(root, { pointerId: 1, clientX: 60, clientY: 40 });
    expect(rectBox()?.textContent).toBe("50 × 30");

    fireEvent.pointerMove(root, { pointerId: 1, clientX: 210, clientY: 110 });
    expect(rectBox()?.textContent).toBe("200 × 100");
  });

  it("小数坐标按 Math.round 取整显示", () => {
    const { root } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 5, clientY: 5 });
    fireEvent.pointerMove(root, { pointerId: 1, clientX: 25.6, clientY: 15.4 });
    expect(rectBox()?.textContent).toBe("21 × 10");
  });
});

describe("CaptureOverlay —— 确认选区", () => {
  it("松开指针把归一化后的矩形四舍五入序列化给 finish_capture，并清空选区", async () => {
    const { root, capture } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 7, clientX: 220, clientY: 210 });
    fireEvent.pointerMove(root, { pointerId: 7, clientX: 120, clientY: 130 });
    fireEvent.pointerUp(root, { pointerId: 7, clientX: 120, clientY: 130 });

    await waitFor(() => {
      expect(callsTo("finish_capture")).toHaveLength(1);
    });
    expect(callsTo("finish_capture")[0].args).toEqual({
      x: 120,
      y: 130,
      width: 100,
      height: 80,
      dpr: 1,
    });
    expect(capture.releaseCapture).toHaveBeenCalledWith(7);
    expect(rectBox()).toBeNull();
    expect(screen.queryByText("选区太小，请重新框选")).not.toBeInTheDocument();
  });

  it("宽度或高度小于 8 视为太小：不调后端并给出中文提示", async () => {
    const { root } = mount();
    drag(root, { x: 30, y: 30 }, { x: 36, y: 200 });

    expect(screen.getByText("选区太小，请重新框选")).toBeInTheDocument();
    expect(callsTo("finish_capture")).toHaveLength(0);
    expect(rectBox()).toBeNull();
    // 让 pointerUp 里 await 的分支落定，确认之后也不会补发命令
    await waitFor(() => expect(callsTo("finish_capture")).toHaveLength(0));
  });

  it("两轴都小于 8 同样判太小", () => {
    const { root } = mount();
    drag(root, { x: 10, y: 10 }, { x: 17, y: 17 });

    expect(screen.getByText("选区太小，请重新框选")).toBeInTheDocument();
    expect(callsTo("finish_capture")).toHaveLength(0);
  });

  it("恰好 8 像素仍算有效选区", async () => {
    const { root } = mount();
    drag(root, { x: 50, y: 60 }, { x: 58, y: 68 });

    await waitFor(() => {
      expect(callsTo("finish_capture")).toHaveLength(1);
    });
    expect(callsTo("finish_capture")[0].args).toEqual({
      x: 50,
      y: 60,
      width: 8,
      height: 8,
      dpr: 1,
    });
  });

  it("finish_capture 失败时提示「截图问答失败：」并清空选区", async () => {
    route("finish_capture", () => Promise.reject("命令未注册"));
    const { root } = mount();
    drag(root, { x: 10, y: 10 }, { x: 110, y: 110 });

    expect(
      await screen.findByText("截图问答失败：命令未注册"),
    ).toBeInTheDocument();
    expect(rectBox()).toBeNull();
  });

  it("再次按下清掉上一条错误提示", () => {
    const { root } = mount();
    drag(root, { x: 10, y: 10 }, { x: 12, y: 12 });
    expect(document.querySelector(".capture-error")).not.toBeNull();

    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 40, clientY: 40 });
    expect(document.querySelector(".capture-error")).toBeNull();
  });

  it("松开后继续移动不再改变尺寸（拖拽已结束）", async () => {
    const { root } = mount();
    drag(root, { x: 10, y: 10 }, { x: 110, y: 110 });
    await waitFor(() => expect(callsTo("finish_capture")).toHaveLength(1));

    fireEvent.pointerMove(root, { pointerId: 1, clientX: 400, clientY: 400 });
    expect(rectBox()).toBeNull();
  });

  it("pointercancel 收起选区且不调 finish_capture", () => {
    const { root } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(root, { pointerId: 1, clientX: 100, clientY: 100 });
    fireEvent.pointerCancel(root, { pointerId: 1 });

    expect(rectBox()).toBeNull();
    expect(callsTo("finish_capture")).toHaveLength(0);
  });
});

describe("CaptureOverlay —— Esc 取消", () => {
  it("Esc 调 cancel_capture 并清空选区", async () => {
    const { root } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(root, { pointerId: 1, clientX: 100, clientY: 100 });

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });

    await waitFor(() => expect(callsTo("cancel_capture")).toHaveLength(1));
    expect(rectBox()).toBeNull();
  });

  it("Esc 重复按键（event.repeat）不重复发命令", async () => {
    mount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", repeat: true }));
    });
    expect(callsTo("cancel_capture")).toHaveLength(0);

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => expect(callsTo("cancel_capture")).toHaveLength(1));
  });

  it("非 Esc 键不发命令", async () => {
    mount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(callsTo("cancel_capture")).toHaveLength(0);
  });

  it("cancel_capture 失败时把错误写进提示条", async () => {
    route("cancel_capture", () => Promise.reject("窗口已销毁"));
    mount();

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });

    expect(await screen.findByText("取消截图失败：窗口已销毁")).toBeInTheDocument();
  });

  it("Esc 取消后拖拽态归零：再 pointerUp 不调 finish_capture", async () => {
    const { root } = mount();
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(root, { pointerId: 1, clientX: 100, clientY: 100 });

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => expect(callsTo("cancel_capture")).toHaveLength(1));

    fireEvent.pointerUp(root, { pointerId: 1, clientX: 100, clientY: 100 });
    expect(callsTo("finish_capture")).toHaveLength(0);
  });

  it("卸载后移除 keydown 监听", async () => {
    const { unmount } = mount();
    unmount();

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(callsTo("cancel_capture")).toHaveLength(0);
  });
});