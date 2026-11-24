import { useCallback, useEffect, useRef, useState } from "react";
import { safeInvoke } from "../lib/tauri";

/** 输入框为空时用来滚动答案区的键。 */
const SCROLL_KEYS = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown"]);

type Props = {
  /** 会话是否在跑；没跑时输入框禁用（后端 ask_question 会拒）。 */
  running: boolean;
};

/**
 * 浮层里的问题输入行：直接把问题打出来问模型，**不经过语音识别**。
 *
 * 为什么放在浮层而不只在设置页：面试时用户盯的就是浮层，切到主窗口打字会
 * 打断注意力、也可能被面试官看到。这里输入完回车即可，答案照常流式上屏。
 *
 * 交互约定：
 * - `Enter` 提交，提交后清空并**保持焦点**（方便连着问）；
 * - `Esc` 先清空输入；输入已空时才让事件冒泡给浮层的「隐藏浮层」；
 * - 输入框里按 `Ctrl+Shift+A`（重新生成）等热键不受影响，因为热键是全局注册的。
 */
export default function QuestionBar({ running }: Props) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [hint, setHint] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const submit = useCallback(async () => {
    const question = text.trim();
    if (!question || busy) return;
    setBusy(true);
    setHint("");
    const result = await safeInvoke("ask_question", { question });
    setBusy(false);
    if (!result.ok) {
      setHint(result.error);
      return;
    }
    setText("");
    // 提交成功给一句极短反馈：浮层空间宝贵，不留常驻提示。
    setHint("已提交");
    window.setTimeout(() => setHint(""), 1500);
    inputRef.current?.focus();
  }, [text, busy]);

  // 浮层显示后自动聚焦到输入框：用户按 Ctrl+Shift+H 摊开浮层就能直接打字。
  useEffect(() => {
    if (running) inputRef.current?.focus();
  }, [running]);

  return (
    <div className="overlay-question-bar">
      <input
        ref={inputRef}
        className="overlay-question-input"
        type="text"
        aria-label="手动提问"
        placeholder={running ? "打字提问，回车发送（不经过语音识别）" : "开始面试后才能提问"}
        value={text}
        disabled={!running}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void submit();
            return;
          }
          if (event.key === "Escape" && text !== "") {
            // 有内容时先清空，不把 Esc 让给「隐藏浮层」——
            // 否则打字打到一半按 Esc 会把整个浮层关掉，白输。
            event.preventDefault();
            event.stopPropagation();
            setText("");
            return;
          }
          // 上下方向键 / 翻页键始终用来滚动答案区。
          //
          // 为什么要专门处理：输入框在会话开始时会自动聚焦（方便直接打字），
          // 而焦点在 <input> 上时方向键只移动光标、不会滚动下方的答案区。
          // 面试里答案通常 150 字、浮层高度有限，往上翻看前文是常见动作，
          // 每次都要先点一下答案区很打断节奏。
          //
          // 不判空的原因（真机验证抓到的）：单行 <input> 里 ArrowUp/Down 没有
          // 光标可移，浏览器会把滚动**原生转发**给最近的滚动容器——也就是说
          // 即使这里不处理，有内容时答案区照样会滚。与其放任浏览器隐式滚动，
          // 不如显式接管：按行高 48px 滚，行为可预期。左右键仍归输入框。
          if (SCROLL_KEYS.has(event.key)) {
            const scroller = document.querySelector(".overlay-answer-body");
            if (scroller) {
              event.preventDefault();
              const page = scroller.clientHeight * 0.9;
              const step =
                event.key === "PageUp" || event.key === "PageDown" ? page : 48;
              const direction =
                event.key === "ArrowUp" || event.key === "PageUp" ? -1 : 1;
              scroller.scrollBy({ top: direction * step, behavior: "smooth" });
            }
          }
        }}
      />
      <button
        type="button"
        className="mini"
        onClick={() => void submit()}
        disabled={!running || busy || !text.trim()}
      >
        {busy ? "…" : "提问"}
      </button>
      {hint && <span className="overlay-question-hint">{hint}</span>}
    </div>
  );
}
