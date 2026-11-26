import { useEffect, useRef } from "react";
import type { RefObject } from "react";
import type { TranscriptLine } from "./types";

/**
 * 转录区贴底滚动：最新一行始终可见，旧行自然往上方溢出。
 *
 * 与 `useAutoScroll`（答案区贴**顶**，最新在上）方向相反——转录是按时间顺序
 * 追加的，新的在下面。
 *
 * 用户向上滚动查看历史时暂停自动贴底，避免新行把正在看的内容顶走；滚回底部
 * 附近后恢复。
 */
export function useStickToBottom(
  ref: RefObject<HTMLDivElement | null>,
  lines: TranscriptLine[],
) {
  const stick = useRef(true);
  const lastCount = useRef(0);

  useEffect(() => {
    const node = ref.current;
    const count = lines.length;
    // 行数变少（会话切换/清空）时重新贴底。
    if (count < lastCount.current) stick.current = true;
    lastCount.current = count;
    if (!node || !stick.current) return;
    // 内容高度变化是异步的（字体、换行），下一帧再贴，否则贴到旧高度。
    const raf = requestAnimationFrame(() => {
      if (node) node.scrollTop = node.scrollHeight;
    });
    return () => cancelAnimationFrame(raf);
  }, [ref, lines.length]);

  function onScroll() {
    const node = ref.current;
    if (!node) return;
    // 距底部 24px 内都算「在底部」，恢复自动贴底。
    stick.current = node.scrollHeight - node.scrollTop - node.clientHeight < 24;
  }

  return { onScroll };
}
