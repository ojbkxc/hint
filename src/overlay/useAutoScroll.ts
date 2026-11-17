import { useEffect, useRef } from "react";
import type { RefObject } from "react";
import type { AnswerState } from "./types";

/** 答案流式输出时保持滚动条贴底；用户手动上滚后暂停自动跟随。 */
export function useAutoScroll(
  ref: RefObject<HTMLDivElement | null>,
  answer: AnswerState,
) {
  const stickToBottom = useRef(true);
  const lastLength = useRef(0);

  useEffect(() => {
    const node = ref.current;
    const length = answer.text.length;
    // 文本变短说明开启了新一轮答案，恢复自动跟随。
    if (length < lastLength.current) stickToBottom.current = true;
    lastLength.current = length;
    if (!node || !stickToBottom.current) return;
    node.scrollTop = node.scrollHeight;
  }, [ref, answer.text]);

  function onScroll() {
    const node = ref.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    stickToBottom.current = distance < 24;
  }

  return { onScroll };
}