import { memo } from "react";
import type { CaptureAnswerState } from "../lib/useCaptureAnswer";

type Props = {
  answer: CaptureAnswerState;
  onDismiss: () => void;
};

/**
 * 截图问答区（浮层内的独立分区）。
 *
 * 刻意复用答案区的类名体系（overlay-answer-toolbar / answer-title /
 * answer-current / answer-text / placeholder / caret / mini ghost），
 * 只在外层加一个分区容器，不另起一套视觉语言：截图答案和面试答案
 * 是两种来源、一样读法。
 */
function CaptureAnswerPanel({ answer, onDismiss }: Props) {
  const hasText = answer.text.trim() !== "";

  return (
    <section className="overlay-capture-answer" aria-label="截图问答">
      <div className="overlay-answer-toolbar">
        <span className="answer-title">
          截图问答
          {answer.streaming && (
            <>
              <i className="streaming-dot" aria-hidden="true" />
              <span className="visually-hidden">正在识别截图</span>
            </>
          )}
          {answer.model !== "" && (
            <span className="answer-count" title={`模型 ${answer.model}`}>
              {answer.model}
            </span>
          )}
        </span>
        <span className="answer-actions">
          <button type="button" className="mini ghost" onClick={onDismiss}>
            关闭
          </button>
        </span>
      </div>

      <div className="overlay-answer-body">
        <div className="answer-current">
          {answer.error !== "" && (
            <div className="answer-error-bar" title={answer.error} role="alert">
              <span className="answer-error-text">{answer.error}</span>
            </div>
          )}

          {hasText ? (
            <p className="answer-text">
              {answer.text}
              {answer.streaming && <span className="caret" />}
            </p>
          ) : answer.error !== "" ? (
            <p className="placeholder">这张截图没有取到答案，可重新框选后再试。</p>
          ) : (
            <p className="placeholder">正在识别截图内容…</p>
          )}
        </div>
      </div>
    </section>
  );
}

/** 电平事件约 30 fps：memo 让跳动只更新状态条，不重排版截图答案。 */
export default memo(CaptureAnswerPanel);
