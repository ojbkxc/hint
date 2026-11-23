import { useEffect, useRef, useState } from "react";
import { safeInvoke } from "./lib/tauri";

type Point = { x: number; y: number };
type Rect = Point & { width: number; height: number };

const MIN_SIZE = 8;

/**
 * 截图选区窗口（M5 预留）。不依赖后端：后端未接线时捕获错误并给出中文提示。
 * 坐标使用 CSS 像素，由后端按窗口缩放换算为物理像素。
 */
export default function CaptureOverlay() {
  const [start, setStart] = useState<Point | null>(null);
  const [current, setCurrent] = useState<Point | null>(null);
  const [notice, setNotice] = useState("");
  const dragging = useRef(false);

  const rect = start && current ? normalizeRect(start, current) : null;

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.repeat) return;
      event.preventDefault();
      void safeInvoke("cancel_capture").then((result) => {
        setStart(null);
        setCurrent(null);
        dragging.current = false;
        setNotice(result.ok ? "" : `取消截图失败：${result.error}`);
      });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  function onPointerDown(event: React.PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    // Pointer capture 在个别宿主（旧 WebView2 / 触控板驱动）上可能缺失：
    // 捕获只是优化（保证移出窗口也能收到 move/up），缺了不影响拖拽本身，
    // 绝不能让它抛错把选区功能整个打死。
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // 忽略：宿主不支持时按无捕获继续。
    }
    dragging.current = true;
    setNotice("");
    setStart({ x: event.clientX, y: event.clientY });
    setCurrent({ x: event.clientX, y: event.clientY });
  }

  function onPointerMove(event: React.PointerEvent<HTMLDivElement>) {
    if (!dragging.current) return;
    setCurrent({ x: event.clientX, y: event.clientY });
  }

  async function finishSelection(event: React.PointerEvent<HTMLDivElement>) {
    if (!dragging.current) return;
    dragging.current = false;
    if (
      typeof event.currentTarget.hasPointerCapture === "function" &&
      event.currentTarget.hasPointerCapture(event.pointerId) &&
      typeof event.currentTarget.releasePointerCapture === "function"
    ) {
      try {
        event.currentTarget.releasePointerCapture(event.pointerId);
      } catch {
        // 同上：释放失败不影响选区结果。
      }
    }
    // 以 pointerup 的坐标为准：快速拖拽时最后一次 move 可能早于 up，
    // 用 state 里的 current 会得到偏小的矩形。up 坐标必然是最新的。
    const area = start ? normalizeRect(start, { x: event.clientX, y: event.clientY }) : null;
    setStart(null);
    setCurrent(null);
    if (!area || area.width < MIN_SIZE || area.height < MIN_SIZE) {
      setNotice("选区太小，请重新拖拽。");
      return;
    }
    const result = await safeInvoke("finish_capture", {
      x: Math.round(area.x),
      y: Math.round(area.y),
      width: Math.round(area.width),
      height: Math.round(area.height),
    });
    if (!result.ok) {
      setNotice(`截图问答尚未接通：${result.error}`);
    }
  }

  return (
    <div
      className="capture-root"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={(event) => void finishSelection(event)}
      onPointerCancel={() => {
        dragging.current = false;
        setStart(null);
        setCurrent(null);
      }}
    >
      {rect && (
        <div
          className="capture-rect"
          style={{
            left: rect.x,
            top: rect.y,
            width: rect.width,
            height: rect.height,
          }}
        >
          <span className="capture-size">
            {Math.round(rect.width)} × {Math.round(rect.height)}
          </span>
        </div>
      )}

      <div className="capture-hint">
        <strong>拖拽框选要提问的区域</strong>
        <span>松开鼠标后截取；Esc 取消。截图问答将在 M5 启用。</span>
        {notice && <span className="capture-error">{notice}</span>}
      </div>
    </div>
  );
}

function normalizeRect(start: Point, current: Point): Rect {
  const x = Math.min(start.x, current.x);
  const y = Math.min(start.y, current.y);
  return {
    x,
    y,
    width: Math.abs(current.x - start.x),
    height: Math.abs(current.y - start.y),
  };
}