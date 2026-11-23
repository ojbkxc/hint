import { useCallback, useEffect, useRef, useState } from "react";
import { isRecord, pickText } from "../lib/data";
import { safeInvoke, safeListen, type InvokeResult } from "../lib/tauri";
import type { SessionState } from "../overlay/types";

const STATUS_EVENT = "hint://status";

const KNOWN_STATES: ReadonlySet<string> = new Set([
  "idle",
  "connecting",
  "listening",
  "ready",
  "connected",
  "error",
]);

function toSessionState(raw: string): SessionState {
  return (KNOWN_STATES.has(raw) ? raw : "idle") as SessionState;
}

type SessionStatus = { state: SessionState; message: string };

export type SessionControl = {
  state: SessionState;
  message: string;
  running: boolean;
  busy: boolean;
  /** 当前正在执行的操作；用于区分「启动中」与「结束中」的按钮文案。 */
  busyAction: "" | "start" | "stop";
  error: string;
  notice: string;
  sessionId: number | null;
  start: () => void;
  stop: () => void;
};

const ACTIVE_STATES = new Set(["connecting", "listening"]);

/**
 * 面试会话控制。状态全部来自 hint://status 事件；
 * 启动失败时命令本身返回中文错误，这里直接展示。
 */
export function useSessionControl(): SessionControl {
  const [status, setStatus] = useState<SessionStatus>({ state: "idle", message: "" });
  const [busyAction, setBusyAction] = useState<"" | "start" | "stop">("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [sessionId, setSessionId] = useState<number | null>(null);
  const disposed = useRef(false);

  useEffect(() => {
    disposed.current = false;
    let active = true;
    let unlisten: (() => void) | null = null;
    void safeListen<unknown>(STATUS_EVENT, (payload) => {
      if (!active || !isRecord(payload)) return;
      const state = pickText(payload, ["state"]);
      const message = pickText(payload, ["message"]);
      if (!state) return;
      setStatus({ state: toSessionState(state), message });
      if (state === "listening") {
        setError("");
        setNotice(message || "面试进行中");
      } else if (state === "idle") {
        setNotice(message || "会话已结束");
        setSessionId(null);
      } else if (state === "error") {
        setError(message || "会话状态异常");
      } else if (message) {
        setNotice(message);
      }
    }).then((fn) => {
      if (!active) {
        fn?.();
        return;
      }
      unlisten = fn;
    });
    return () => {
      active = false;
      disposed.current = true;
      unlisten?.();
    };
  }, []);

  const start = useCallback(async () => {
    setBusyAction("start");
    setError("");
    setNotice("");
    // 命令要冷启语音引擎（约 10–30 秒），先切到 connecting，
    // 让界面立刻显示真实阶段，而不是一个没有反馈的「开始面试」。
    setStatus({ state: "connecting", message: "正在启动语音引擎（首次加载较慢，请稍候）…" });
    let result: InvokeResult<unknown>;
    try {
      result = await safeInvoke<unknown>("start_session");
    } finally {
      if (!disposed.current) setBusyAction("");
    }
    if (disposed.current) return;
    if (!result.ok) {
      setError(result.error);
      // 启动失败时后端会补发 status=error；若命令先于事件返回，这里兜底。
      setStatus((current) =>
        current.state === "error" ? current : { state: "error", message: result.error },
      );
      return;
    }
    const id =
      typeof result.data === "number"
        ? result.data
        : Number(result.data);
    if (Number.isFinite(id) && id > 0) setSessionId(id);
    setNotice("会话已启动");
    setStatus((current) =>
      // 启动期间后端已发 listening 或 error（命令与事件竞态）时以先到为准；
      // 只保护 listening 会把 error 态覆盖回 connecting，错误提示被吞。
      current.state === "listening" || current.state === "error"
        ? current
        : { state: "connecting", message: "正在启动语音引擎（首次加载较慢，请稍候）…" },
    );
  }, []);

  const stop = useCallback(async () => {
    setBusyAction("stop");
    setError("");
    setNotice("");
    const result = await safeInvoke<unknown>("stop_session");
    if (disposed.current) return;
    setBusyAction("");
    if (!result.ok) {
      setError(`结束会话失败：${result.error}`);
      return;
    }
    setStatus({ state: "idle", message: "会话已结束" });
    setSessionId(null);
    setNotice("会话已结束，可在「会话复盘」里回看转录与问答。");
  }, []);

  const running = ACTIVE_STATES.has(status.state);
  const busy = busyAction !== "";

  return {
    state: status.state,
    message: status.message,
    running,
    busy,
    busyAction,
    error,
    notice,
    sessionId,
    start: () => void start(),
    stop: () => void stop(),
  };
}