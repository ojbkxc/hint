import { useEffect, useRef, useState } from "react";
import type { AnswerState } from "./types";

/**
 * 等待首字期间已过的秒数；不在等待时返回 0。
 *
 * ## 为什么需要
 *
 * 真机数据（同一份代码）：快的会话 ttft 1.7 秒，但有一次面试连续 19 条都在
 * 8～69 秒之间（其中一条 68.9 秒）。那种情况下浮层只显示「正在等待模型输出…」，
 * 用户**无法区分「在等」与「卡死了」**——面试中他只有几秒决定要不要放弃这条。
 *
 * 显示秒数让人能自己判断；超过阈值再补一句「响应较慢」，说明这不是本地卡住。
 *
 * ## 为什么计时从 streaming 变 true 时开始
 *
 * `streaming` 由后端事件驱动，正好是「请求已发出」的时刻；用组件挂载时间当起点
 * 会把上一次的等待时长算进来。
 */
export function useWaitSeconds(answer: AnswerState): number {
  // 只在这种状态下计时：正在流式，但一个字都还没到。
  const waiting = answer.streaming && answer.text === "";
  const [seconds, setSeconds] = useState(0);
  const startedAt = useRef<number | null>(null);

  useEffect(() => {
    if (!waiting) {
      startedAt.current = null;
      setSeconds(0);
      return;
    }
    startedAt.current = Date.now();
    setSeconds(0);
    // 1 秒一跳：秒数要看起来在动，但不必更密。
    const timer = window.setInterval(() => {
      if (startedAt.current !== null) {
        setSeconds(Math.floor((Date.now() - startedAt.current) / 1000));
      }
    }, 1000);
    return () => window.clearInterval(timer);
  }, [waiting]);

  return waiting ? seconds : 0;
}
