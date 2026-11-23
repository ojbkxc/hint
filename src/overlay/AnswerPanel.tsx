import { memo, useEffect, useRef } from "react";
import type { AnswerHistoryEntry, AnswerState } from "./types";
import { useAutoScroll } from "./useAutoScroll";

type Props = {
  answer: AnswerState;
  history: AnswerHistoryEntry[];
  /** 本场已归档的问答总数（history 会被截断，不能用它的长度代替）。 */
  answeredTotal: number;
  onRegenerate: () => void;
  onClear: () => void;
  onDismissError: () => void;
};

function AnswerPanel({
  answer,
  history,
  answeredTotal,
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
      // 焦点在转录面板上时把方向键让回去：那块也是可滚动区域，
      // 全局劫持会让键盘用户永远翻不动转录。
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest(".overlay-transcript") !== null
      ) {
        return;
      }
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
  const answeredCount = answeredTotal + (hasText ? 1 : 0);

  return (
    <div className="overlay-answer">
      <div className="overlay-answer-toolbar">
        <span className="answer-title">
          答案
          {answer.streaming && (
            <>
              <i className="streaming-dot" aria-hidden="true" />
              <span className="visually-hidden">正在生成</span>
            </>
          )}
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
          ) : answer.muted ? (
            <p className="placeholder">
              本轮答案已清空，模型后续生成的内容不再显示。
              <br />
              需要完整答案时点「重新生成」。
            </p>
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
            {[...history].reverse().map((item) => (
              <section className="answer-block past" key={item.id}>
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

/**
 * 电平事件约 30 fps，全量重渲染会把长答案文本一起重算。
 * props 只在答案/历史/计数变化时改变（回调由 Overlay 用 useCallback 稳定），
 * memo 让电平跳动只更新 StatusBar。
 */
export default memo(AnswerPanel);
