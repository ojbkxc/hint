import { useEffect, useRef } from "react";
import type { AnswerState } from "./types";
import { useAutoScroll } from "./useAutoScroll";

type Props = {
  answer: AnswerState;
  onRegenerate: () => void;
  onClear: () => void;
};

export default function AnswerPanel({ answer, onRegenerate, onClear }: Props) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const { onScroll } = useAutoScroll(bodyRef, answer);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const node = bodyRef.current;
      if (!node) return;
      if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
      const step = 64;
      const delta = event.key === "ArrowUp" ? -step : step;
      const max = node.scrollHeight - node.clientHeight;
      if (max <= 0) return;
      const next = Math.min(max, Math.max(0, node.scrollTop + delta));
      if (next !== node.scrollTop) {
        event.preventDefault();
        node.scrollTop = next;
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const hasText = answer.text.trim() !== "";

  return (
    <div className="overlay-answer">
      <div className="overlay-answer-toolbar">
        <span className="answer-title">
          答案
          {answer.streaming && <i className="streaming-dot" title="正在生成" />}
        </span>
        <span className="answer-actions">
          {hasText && (
            <button type="button" className="mini ghost" onClick={onClear}>
              清空
            </button>
          )}
          <button type="button" className="mini" onClick={onRegenerate}>
            重新生成
          </button>
        </span>
      </div>

      <div className="overlay-answer-body" ref={bodyRef} onScroll={onScroll}>
        {hasText ? (
          <p className="answer-text">
            {answer.text}
            {answer.streaming && <span className="caret" />}
          </p>
        ) : answer.streaming ? (
          <p className="placeholder">正在等待模型输出…</p>
        ) : (
          <p className="placeholder">
            检测到面试官提问后，答案会流式出现在这里。
            <br />
            也可以点「重新生成」对最新问题再要一次回答。
          </p>
        )}
      </div>
    </div>
  );
}