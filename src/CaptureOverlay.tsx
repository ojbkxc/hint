import { useCallback, useEffect, useRef, useState } from "react";
import { safeInvoke } from "./lib/tauri";

type Point = { x: number; y: number };
type Rect = Point & { width: number; height: number };

const MIN_SIZE = 8;
/** 错误气泡停留时长：够读完一句中文提示，又不至于挡着下一次框选。 */
const NOTICE_MS = 2500;

/**
 * 截图选区窗口（M5）。
 *
 * 坐标一律用 CSS 像素上报，并附上 devicePixelRatio 让后端换算成物理像素：
 * 高 DPI 屏上只给 CSS 像素，截出来的图会缩到一半区域。
 * 后端负责隐藏本窗口（成功路径不再回头改 UI）；前端只管本地状态。
 */
export default function CaptureOverlay() {
  const [start, setStart] = useState<Point | null>(null);
  const [current, setCurrent] = useState<Point | null>(null);
  const [notice, setNotice] = useState("");
  const dragging = useRef(false);
  const noticeTimer = useRef<number | null>(null);

  const rect = start && current ? normalizeRect(start, current) : null;

  // 错误气泡到点自动消失：不挡住下一次框选，也不需要用户手动关。
  useEffect(() => {
    if (notice === "") return;
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => {
      noticeTimer.current = null;
      setNotice("");
    }, NOTICE_MS);
    return () => {
      if (noticeTimer.current !== null) {
        window.clearTimeout(noticeTimer.current);
        noticeTimer.current = null;
      }
    };
  }, [notice]);

  /** 复位拖拽态：Esc / 右键 / pointercancel 共用。 */
  const resetDrag = useCallback(() => {
    dragging.current = false;
    setStart(null);
    setCurrent(null);
  }, []);

  const cancel = useCallback(async () => {
    resetDrag();
    const result = await safeInvoke("cancel_capture");
    setNotice(result.ok ? "" : `取消截图失败：${result.error}`);
  }, [resetDrag]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.repeat) return;
      event.preventDefault();
      void cancel();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancel]);

  function onPointerDown(event: React.PointerEvent<HTMLDivElement>) {
    // 只有左键开始框选；右键交给 onContextMenu 当「取消」。
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
    resetDrag();
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
    const area = start
      ? normalizeRect(start, { x: event.clientX, y: event.clientY })
      : null;
    if (!area || area.width < MIN_SIZE || area.height < MIN_SIZE) {
      setNotice("选区太小，请重新框选");
      return;
    }
    const result = await safeInvoke("finish_capture", {
      x: Math.round(area.x),
      y: Math.round(area.y),
      width: Math.round(area.width),
      height: Math.round(area.height),
      dpr: window.devicePixelRatio || 1,
    });
    if (!result.ok) {
      setNotice(`截图问答失败：${result.error}`);
      return;
    }
    // 成功：后端已经把选区窗口隐藏掉，前端不弹任何提示。
    setNotice("");
  }

  return (
    <div
      className="capture-root"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={(event) => void finishSelection(event)}
      onPointerCancel={resetDrag}
      onContextMenu={(event) => {
        event.preventDefault();
        void cancel();
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
        <span>松开鼠标后截取；Esc 或右键取消。</span>
        {notice !== "" && (
          <span className="capture-error" role="alert">
            {notice}
          </span>
        )}
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
