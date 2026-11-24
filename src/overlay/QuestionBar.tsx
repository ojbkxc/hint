import { useCallback, useEffect, useRef, useState } from "react";
import { safeInvoke } from "../lib/tauri";

/** 方向键的滚动步进（像素）。 */
const STEP_PX = 48;

/** 用来自动滚动答案区的键。 */
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

  /**
   * 上下方向键 / 翻页键滚动答案区。
   *
   * 为什么放在 window 的**捕获阶段**而不是 input 的 onKeyDown：
   * 真机实测过，React 合成事件里调 preventDefault 拦不住浏览器的原生滚动
   * （捕获阶段读到的 `defaultPrevented` 仍是 false，答案区照样滚了一屏的 80%）。
   * 捕获阶段在浏览器执行默认动作之前，才能真正接管。
   *
   * 为什么不管左右键：单行输入框里左右键要移动光标，抢了就没法编辑。
   */
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!SCROLL_KEYS.has(event.key)) return;
      // 只管我们自己的输入框：焦点在别处（答案区、按钮）时不干预。
      if (document.activeElement !== inputRef.current) return;
      const scroller = document.querySelector(".overlay-answer-body");
      if (!scroller || scroller.scrollHeight <= scroller.clientHeight) return;
      event.preventDefault();
      event.stopPropagation();
      const page = scroller.clientHeight * 0.9;
      const step =
        event.key === "PageUp" || event.key === "PageDown" ? page : STEP_PX;
      const direction = event.key === "ArrowUp" || event.key === "PageUp" ? -1 : 1;
      scroller.scrollBy({ top: direction * step, behavior: "auto" });
    }
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

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
