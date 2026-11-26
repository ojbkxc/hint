import { memo, useEffect, useRef } from "react";
import type { AnswerHistoryEntry, AnswerState } from "./types";
import { useAutoScroll } from "./useAutoScroll";
import { useWaitSeconds } from "./useWaitSeconds";

/** 等这么久还没首字就提示「响应较慢」。
 *
 * 取 10 秒的依据：真机正常会话 ttft 在 1.7～3.4 秒，而慢的那场从 8.4 秒起步。
 * 10 秒落在两者之间——正常会话永远不会看到这条提示，异常时才出现。
 */
const SLOW_HINT_SECONDS = 10;

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
  const waitSeconds = useWaitSeconds(answer);

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
            // 显示秒数：真机有过连续 19 条 ttft 在 8～69 秒的面试，只写「正在等待」
            // 的话用户分不清是在等还是卡死了，而面试中他只有几秒决定要不要放弃。
            <p className={waitSeconds >= SLOW_HINT_SECONDS ? "placeholder slow-hint" : "placeholder"}>
              正在等待模型输出
              {waitSeconds > 0 ? `（已等待 ${waitSeconds} 秒）` : "…"}
              {waitSeconds >= SLOW_HINT_SECONDS && (
                <>
                  <br />
                  模型响应较慢，通常与网络或服务端排队有关；可以再等一会，或点「重新生成」。
                </>
              )}
            </p>
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
              检测到提问后，答案会流式出现在这里。
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
