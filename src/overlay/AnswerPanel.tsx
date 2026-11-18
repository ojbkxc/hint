import { useEffect, useRef } from "react";
import type { AnswerHistoryEntry, AnswerState } from "./types";
import { useAutoScroll } from "./useAutoScroll";

type Props = {
  answer: AnswerState;
  history: AnswerHistoryEntry[];
  onRegenerate: () => void;
  onClear: () => void;
  onDismissError: () => void;
};

export default function AnswerPanel({
  answer,
  history,
  onRegenerate,
  onClear,
  onDismissError,
}: Props) {
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
  const answeredCount = history.length + (hasText ? 1 : 0);

  return (
    <div className="overlay-answer">
      <div className="overlay-answer-toolbar">
        <span className="answer-title">
          答案
          {answer.streaming && <i className="streaming-dot" title="正在生成" />}
          {answeredCount > 0 && (
            <span className="answer-count">本场已答 {answeredCount} 题</span>
          )}
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
        <div className="answer-current">
          {(answer.question !== "" || answer.pending > 0) && (
            <div className="overlay-question-row">
              <span className="overlay-question" title={answer.question}>
                {answer.question !== "" ? `问：${answer.question}` : "当前问题"}
              </span>
              {answer.pending > 0 && (
                <span
                  className="pending-badge"
                  title={`还有 ${answer.pending} 个问题等待回答`}
                >
                  排队 {answer.pending}
                </span>
              )}
            </div>
          )}

          {answer.error !== "" && (
            <div className="answer-error-bar" title={answer.error} role="alert">
              <span className="answer-error-text">{answer.error}</span>
              <span className="answer-error-actions">
                {answer.retryable && (
                  <button type="button" className="mini" onClick={onRegenerate}>
                    重试
                  </button>
                )}
                <button type="button" className="mini ghost" onClick={onDismissError}>
                  关闭
                </button>
              </span>
            </div>
          )}

          {hasText ? (
            <p className="answer-text">
              {answer.text}
              {answer.streaming && <span className="caret" />}
            </p>
          ) : answer.streaming ? (
            <p className="placeholder">正在等待模型输出…</p>
          ) : answer.error !== "" ? (
            <p className="placeholder">本次没有生成内容，可按上方提示处理后重试。</p>
          ) : (
            <p className="placeholder">
              检测到面试官提问后，答案会流式出现在这里。
              <br />
              也可以点「重新生成」对最新问题再要一次回答。
            </p>
          )}
        </div>

        {history.length > 0 && (
          <div className="answer-history-timeline">
            <p className="history-caption">更早的回答</p>
            {[...history].reverse().map((item, index) => (
              <section className="answer-block past" key={`${item.question}-${index}`}>
                <p className="block-question" title={item.question}>
                  {item.question !== "" ? `问：${item.question}` : "（未记录问题）"}
                </p>
                <p className="block-answer" title={item.text}>
                  {item.text || "（空答案）"}
                </p>
              </section>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}