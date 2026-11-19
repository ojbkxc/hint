import { useCallback, useEffect, useRef, useState } from "react";
import AnswerPanel from "./overlay/AnswerPanel";
import StatusBar from "./overlay/StatusBar";
import TranscriptPanel from "./overlay/TranscriptPanel";
import { useOverlayEvents } from "./lib/useOverlayEvents";

const ACTIVE_STATES = new Set(["connected", "listening", "ready"]);

type Toast = { level: "info" | "error"; message: string; key: number };

const INFO_MS = 6000;
const ERROR_MS = 10000;

export default function Overlay() {
  const {
    transcript,
    answer,
    answerHistory,
    levels,
    speech,
    status,
    notice,
    hideOverlay,
    regenerate,
    clearAnswer,
    dismissAnswerError,
  } = useOverlayEvents();
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

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.repeat) return;
      event.preventDefault();
      void handleHide();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleHide]);

  async function handleRegenerate() {
    setLocalError("");
    const error = await regenerate();
    if (error) setLocalError(`重新生成失败：${error}`);
  }

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
      </header>

      <TranscriptPanel lines={transcript} />

      <AnswerPanel
        answer={answer}
        history={answerHistory}
        onRegenerate={handleRegenerate}
        onClear={() => {
          setLocalError("");
          clearAnswer();
        }}
        onDismissError={dismissAnswerError}
      />

      <StatusBar
        levels={levels}
        model={answer.model}
        ttftMs={answer.ttftMs}
        streaming={answer.streaming}
        speech={speech}
      />

      {toast && (
        <button
          key={toast.key}
          type="button"
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