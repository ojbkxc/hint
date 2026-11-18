import { useEffect, useRef } from "react";
import type { RefObject } from "react";
import type { AnswerState } from "./types";

/**
 * 最新答案在顶部：新一轮开始与流式输出时保持顶部可见；
 * 用户主动向下滚动后暂停自动定位，避免抢走正在回看的历史。
 */
export function useAutoScroll(
  ref: RefObject<HTMLDivElement | null>,
  answer: AnswerState,
) {
  const stickToTop = useRef(true);
  const lastLength = useRef(0);
  const lastQuestion = useRef("");

  useEffect(() => {
    const node = ref.current;
    const length = answer.text.length;
    // 文本变短或问题变化说明开启新一轮，重新贴顶。
    const newRound =
      length < lastLength.current || answer.question !== lastQuestion.current;
    if (newRound) stickToTop.current = true;
    lastLength.current = length;
    lastQuestion.current = answer.question;
    if (!node || !stickToTop.current) return;
    if (node.scrollTop !== 0) node.scrollTop = 0;
  }, [ref, answer.text, answer.question]);

  function onScroll() {
    const node = ref.current;
    if (!node) return;
    stickToTop.current = node.scrollTop < 24;
  }

  return { onScroll };
}