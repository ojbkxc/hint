import { useCallback, useEffect, useState } from "react";
import AnswerPanel from "./overlay/AnswerPanel";
import StatusBar from "./overlay/StatusBar";
import TranscriptPanel from "./overlay/TranscriptPanel";
import { useOverlayEvents } from "./lib/useOverlayEvents";

const ACTIVE_STATES = new Set(["connected", "listening", "ready"]);

export default function Overlay() {
  const { transcript, answer, levels, status, notice, hideOverlay, regenerate, clearAnswer } =
    useOverlayEvents();
  const [localNotice, setLocalNotice] = useState("");
  const [answerError, setAnswerError] = useState("");
  const toast = localNotice || notice || answerError;

  const handleHide = useCallback(async () => {
    const error = await hideOverlay();
    if (error) setLocalNotice(`隐藏浮层失败：${error}`);
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
    setAnswerError("");
    const error = await regenerate();
    if (error) setAnswerError(`重新生成失败：${error}`);
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
        onRegenerate={handleRegenerate}
        onClear={() => {
          setAnswerError("");
          clearAnswer();
        }}
      />

      <StatusBar
        levels={levels}
        model={answer.model}
        ttftMs={answer.ttftMs}
        streaming={answer.streaming}
      />

      {toast && <div className="overlay-toast">{toast}</div>}
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