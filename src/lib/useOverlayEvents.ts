import { useCallback, useEffect, useRef, useState } from "react";
import { HINT_EVENTS, MAX_TRANSCRIPT_LINES } from "../overlay/types";
import type {
  AnswerState,
  LevelsState,
  StatusState,
  TranscriptLine,
} from "../overlay/types";
import { isRecord, numberValue, pickText } from "./data";
import { safeInvoke, safeListen } from "./tauri";

type OverlayListenerState = {
  transcript: TranscriptLine[];
  answer: AnswerState;
  levels: LevelsState;
  status: StatusState;
  /** 非会话提示（模型下载、热键注册等）；不参与会话状态判断。 */
  notice: string;
};

const INITIAL_STATE: OverlayListenerState = {
  transcript: [],
  answer: { text: "", streaming: false, done: false, model: "", ttftMs: null },
  levels: { mic: 0, system: 0 },
  status: { state: "idle", message: "等待连接" },
  notice: "",
};

/**
 * 浮层全部后端事件订阅集中在这里。
 * 事件不存在时保持初始状态，不抛异常、不白屏。
 */
export function useOverlayEvents() {
  const [state, setState] = useState<OverlayListenerState>(INITIAL_STATE);
  const nextId = useRef(1);

  useEffect(() => {
    let disposed = false;
    const disposers: Array<() => void> = [];

    const patch = (next: Partial<OverlayListenerState>) => {
      if (!disposed) setState((current) => ({ ...current, ...next }));
    };

    const register = (event: string, apply: (payload: unknown) => void) => {
      void safeListen<unknown>(event, (payload) => {
        if (!disposed) apply(payload);
      }).then((unlisten) => {
        if (!unlisten) return;
        if (disposed) unlisten();
        else disposers.push(unlisten);
      });
    };

    register(HINT_EVENTS.transcript, (payload) => {
      if (!isRecord(payload)) return;
      const channel = payload.channel === "me" ? "me" : "interviewer";
      const text = pickText(payload, ["text", "transcript"]);
      if (!text.trim()) return;
      const line: TranscriptLine = {
        id: nextId.current++,
        channel,
        text,
        isEcho: payload.isEcho === true,
      };
      setState((current) => {
        if (disposed) return current;
        const transcript = [...current.transcript, line].slice(-MAX_TRANSCRIPT_LINES);
        return { ...current, transcript };
      });
    });

    register(HINT_EVENTS.answerDelta, (payload) => {
      if (!isRecord(payload)) return;
      const text = pickText(payload, ["text", "delta"]);
      const forcedReset = payload.reset === true;
      setState((current) => {
        if (disposed) return current;
        // 新一轮流式输出开始：清空上一轮答案。
        const reset =
          forcedReset || current.answer.done || !current.answer.streaming;
        const base = reset ? "" : current.answer.text;
        return {
          ...current,
          answer: {
            ...current.answer,
            text: base + text,
            streaming: true,
            done: false,
          },
        };
      });
    });

    register(HINT_EVENTS.answerDone, (payload) => {
      if (!isRecord(payload)) return;
      const finalText = pickText(payload, ["text", "answer"]);
      const model = pickText(payload, ["model"]);
      const ttft = numberValue(payload.ttftMs ?? payload.ttft_ms ?? payload.ttft);
      // 后端结束会话 / 判定失败时会发空 text + 空 model，视为清空答案。
      const isClear = finalText === "" && model === "";
      setState((current) => {
        if (disposed) return current;
        if (isClear) {
          return {
            ...current,
            answer: { text: "", streaming: false, done: true, model: "", ttftMs: null },
          };
        }
        return {
          ...current,
          answer: {
            text: finalText !== "" ? finalText : current.answer.text,
            streaming: false,
            done: true,
            model: model || current.answer.model,
            ttftMs: ttft ?? current.answer.ttftMs,
          },
        };
      });
    });

    register(HINT_EVENTS.levels, (payload) => {
      if (!isRecord(payload)) return;
      const mic = Math.min(1, Math.max(0, numberValue(payload.mic) ?? 0));
      const system = Math.min(1, Math.max(0, numberValue(payload.system) ?? 0));
      patch({ levels: { mic, system } });
    });

    register(HINT_EVENTS.status, (payload) => {
      if (!isRecord(payload)) return;
      patch({
        status: {
          state: pickText(payload, ["state"]) || "unknown",
          message: pickText(payload, ["message"]),
        },
      });
    });

    register(HINT_EVENTS.notice, (payload) => {
      if (!isRecord(payload)) return;
      const message = pickText(payload, ["message"]);
      if (message) patch({ notice: message });
    });

    return () => {
      disposed = true;
      for (const unlisten of disposers) unlisten();
    };
  }, []);

  const hideOverlay = useCallback(async () => {
    const result = await safeInvoke("hide_overlay");
    return result.ok ? null : result.error;
  }, []);

  const regenerate = useCallback(async () => {
    const result = await safeInvoke("regenerate_answer");
    if (!result.ok) return result.error;
    setState((current) => ({
      ...current,
      answer: { text: "", streaming: true, done: false, model: "", ttftMs: null },
    }));
    return null;
  }, []);

  const clearAnswer = useCallback(() => {
    setState((current) => ({
      ...current,
      answer: { text: "", streaming: false, done: false, model: "", ttftMs: null },
    }));
  }, []);

  return { ...state, hideOverlay, regenerate, clearAnswer };
}