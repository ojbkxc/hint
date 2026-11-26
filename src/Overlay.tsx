import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import AnswerPanel from "./overlay/AnswerPanel";
import CaptureAnswerPanel from "./overlay/CaptureAnswerPanel";
import StatusBar from "./overlay/StatusBar";
import TranscriptPanel from "./overlay/TranscriptPanel";
import { useCaptureAnswer } from "./lib/useCaptureAnswer";
import { useOverlayEvents } from "./lib/useOverlayEvents";
import { safeInvoke } from "./lib/tauri";
import QuestionBar from "./overlay/QuestionBar";

const ACTIVE_STATES = new Set(["connected", "listening", "ready"]);

type Toast = { level: "info" | "error"; message: string; key: number };

const INFO_MS = 6000;
const ERROR_MS = 10000;

export default function Overlay() {
  const {
    transcript,
    answer,
    answerHistory,
    answeredTotal,
    levels,
    speech,
    status,
    notice,
    hideOverlay,
    regenerate,
    clearAnswer,
    dismissAnswerError,
  } = useOverlayEvents();

  // 会话开始 / 结束。浮层是面试时唯一看得见的窗口（主窗口收进托盘是常态），
  // 要开始面试却必须切回主窗口或右键托盘去找入口，等于把最常用的动作藏起来。
  const [sessionBusy, setSessionBusy] = useState(false);
  // 状态取值见 overlay/types.ts 的 SessionState：没有 "running"，
  // 进行中是 listening/ready/connected，启动过程是 connecting——按钮都该显示「结束」，
  // 否则启动那几秒会显示「开始面试」，用户会再点一次。
  const running =
    ACTIVE_STATES.has(status.state) || status.state === "connecting";
  const toggleSession = useCallback(async () => {
    setSessionBusy(true);
    try {
      if (running) {
        await safeInvoke("stop_session");
      } else {
        await safeInvoke("start_session");
      }
    } finally {
      setSessionBusy(false);
    }
  }, [running]);
  // 截图问答与面试问答是两条独立事件流：各自的状态、各自的分区，互不覆盖。
  const {
    text: captureText,
    streaming: captureStreaming,
    error: captureError,
    model: captureModel,
    dismiss: dismissCaptureAnswer,
  } = useCaptureAnswer();
  // memo 化的 CaptureAnswerPanel 靠引用比较：电平事件重渲染时不要换新对象。
  const captureAnswer = useMemo(
    () => ({
      text: captureText,
      streaming: captureStreaming,
      error: captureError,
      model: captureModel,
    }),
    [captureText, captureStreaming, captureError, captureModel],
  );
  // 没有任何截图内容时不占位：浮层版式与改动前完全一致（不挤压转录区）。
  const hasCaptureAnswer =
    captureStreaming || captureError !== "" || captureText.trim() !== "";
  const [localError, setLocalError] = useState("");
  const [toast, setToast] = useState<Toast | null>(null);
  const toastKey = useRef(0);
  const dismissTimer = useRef<number | null>(null);

  const showToast = useCallback((level: "info" | "error", message: string) => {
    toastKey.current += 1;
    setToast({ level, message, key: toastKey.current });
  }, []);

  const dismissToast = useCallback(() => {
    if (dismissTimer.current !== null) {
      window.clearTimeout(dismissTimer.current);
      dismissTimer.current = null;
    }
    setToast(null);
  }, []);

  // 后端提示（模型下载、热键注册等）按 level 决定样式与停留时间。
  useEffect(() => {
    if (notice.message) {
      showToast(notice.level === "error" ? "error" : "info", notice.message);
    }
  }, [notice, showToast]);

  useEffect(() => {
    if (!localError) return;
    showToast("error", localError);
    setLocalError("");
  }, [localError, showToast]);

  useEffect(() => {
    if (!toast) return;
    if (dismissTimer.current !== null) window.clearTimeout(dismissTimer.current);
    dismissTimer.current = window.setTimeout(
      () => {
        dismissTimer.current = null;
        setToast(null);
      },
      toast.level === "error" ? ERROR_MS : INFO_MS,
    );
    return () => {
      if (dismissTimer.current !== null) {
        window.clearTimeout(dismissTimer.current);
        dismissTimer.current = null;
      }
    };
  }, [toast]);

  const handleHide = useCallback(async () => {
    const error = await hideOverlay();
    if (error) setLocalError(`隐藏浮层失败：${error}`);
  }, [hideOverlay]);

  // 回调必须稳定：AnswerPanel / TranscriptPanel 是 memo 组件，
  // 每次电平事件都换新函数会让 memo 完全失效。
  const handleRegenerate = useCallback(async (): Promise<void> => {
    setLocalError("");
    const error = await regenerate();
    if (error) setLocalError(`重新生成失败：${error}`);
  }, [regenerate]);

  const handleClear = useCallback(() => {
    setLocalError("");
    clearAnswer();
  }, [clearAnswer]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.repeat) return;
      event.preventDefault();
      void handleHide();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleHide]);

  return (
    <div className="overlay-root">
      <header className="overlay-header" data-tauri-drag-region>
        <span className="overlay-brand" data-tauri-drag-region>
          hint
        </span>
        <span
          className={`overlay-status ${statusTone(status.state)}`}
          data-tauri-drag-region
          title={status.message || status.state}
        >
          <i className="dot" />
          {status.message || stateLabel(status.state)}
          {answer.ttftMs !== null && (
            <em className="header-latency">首字 {(answer.ttftMs / 1000).toFixed(1)}s</em>
          )}
        </span>
        {/* 面试中浮层往往是屏幕上唯一看得到的窗口，开始/结束不该逼用户切回主窗口
            或右键托盘。按钮与主窗口「开始面试 / 结束面试」调同一组命令。 */}
        <button
          type="button"
          className={`overlay-session-btn ${running ? "is-running" : ""}`}
          onClick={() => void toggleSession()}
          disabled={sessionBusy}
          title={running ? "结束本次面试（同主窗口「结束面试」）" : "开始面试（同主窗口「开始面试」）"}
        >
          {sessionBusy ? "…" : running ? "结束面试" : "开始面试"}
        </button>
      </header>

      <TranscriptPanel lines={transcript} />

      <AnswerPanel
        answer={answer}
        history={answerHistory}
        answeredTotal={answeredTotal}
        onRegenerate={handleRegenerate}
        onClear={handleClear}
        onDismissError={dismissAnswerError}
      />

      {hasCaptureAnswer && (
        <CaptureAnswerPanel
          answer={captureAnswer}
          onDismiss={dismissCaptureAnswer}
        />
      )}

      {/* 手动提问行：识别不准时直接在浮层里打字问，不用切窗口。
          放在答案区之后、状态栏之前——既显眼又不挤占答案空间。 */}
      <QuestionBar running={ACTIVE_STATES.has(status.state)} />

      <StatusBar
        levels={levels}
        // 优先显示最近一次答题实际用的模型；还没答过题时回落显示配置里选定的模型。
        // 旧实现只用 answer.model，导致没答题时永远显示「未连接模型」——
        // 那文案会让人以为接口没连上（真机反馈过）。
        model={answer.model || answer.configuredModel}
        ttftMs={answer.ttftMs}
        streaming={answer.streaming}
        speech={speech}
      />

      {toast && (
        <button
          key={toast.key}
          type="button"
          role={toast.level === "error" ? "alert" : "status"}
          aria-live={toast.level === "error" ? "assertive" : "polite"}
          className={`overlay-toast toast-${toast.level}`}
          onClick={dismissToast}
          title={`${toast.message}（点击关闭）`}
        >
          {toast.message}
        </button>
      )}
    </div>
  );
}

function statusTone(state: string): string {
  if (ACTIVE_STATES.has(state)) return "tone-ok";
  if (state === "error") return "tone-error";
  if (state === "connecting") return "tone-pending";
  return "tone-idle";
}

function stateLabel(state: string): string {
  const map: Record<string, string> = {
    idle: "等待连接",
    connecting: "连接中",
    listening: "监听中",
    ready: "已就绪",
    connected: "已连接",
    error: "连接异常",
  };
  return map[state] ?? state;
}