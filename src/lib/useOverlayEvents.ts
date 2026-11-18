import { useCallback, useEffect, useRef, useState } from "react";
import { HINT_EVENTS, MAX_TRANSCRIPT_LINES } from "../overlay/types";
import type {
  AnswerHistoryEntry,
  AnswerState,
  LevelsState,
  NoticeState,
  StatusState,
  TranscriptLine,
} from "../overlay/types";
import { isRecord, numberValue, pickText } from "./data";
import { safeInvoke, safeListen } from "./tauri";
import { normalizeOpacity, normalizeSettings } from "../settings/types";

const MAX_ANSWER_HISTORY = 10;

type OverlayListenerState = {
  transcript: TranscriptLine[];
  answer: AnswerState;
  /** 本场已回答过的问答（不含当前这一轮），按时间先后追加。 */
  answerHistory: AnswerHistoryEntry[];
  levels: LevelsState;
  status: StatusState;
  /** 非会话提示（模型下载、热键注册等）；不参与会话状态判断。 */
  notice: NoticeState;
};

const EMPTY_ANSWER: AnswerState = {
  text: "",
  streaming: false,
  done: false,
  model: "",
  ttftMs: null,
  question: "",
  pending: 0,
  error: "",
  retryable: false,
};

const INITIAL_STATE: OverlayListenerState = {
  transcript: [],
  answer: EMPTY_ANSWER,
  answerHistory: [],
  levels: { mic: 0, system: 0 },
  status: { state: "idle", message: "等待连接" },
  notice: { level: "", message: "" },
};

/** 浮层背景不透明度：只改 CSS 变量，避免触发重渲染打断流式输出。 */
function applyOverlayOpacity(value: number) {
  document.documentElement.style.setProperty("--overlay-alpha", String(value));
}

/**
 * 浮层全部后端事件订阅集中在这里。
 * 事件不存在时保持初始状态，不抛异常、不白屏。
 */
export function useOverlayEvents() {
  const [state, setState] = useState<OverlayListenerState>(INITIAL_STATE);
  const nextId = useRef(1);

  useEffect(() => {
    // 启动时读取已保存的不透明度；失败保持 CSS 默认值，不在浮层弹错误。
    void safeInvoke<unknown>("get_settings").then((result) => {
      if (!result.ok) return;
      applyOverlayOpacity(normalizeSettings(result.data).overlayOpacity);
    });
  }, []);

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
      const payloadQuestion = pickText(payload, ["question"]);
      const pendingValue = numberValue(payload.pending);
      const payloadPending =
        pendingValue === null ? null : Math.max(0, Math.round(pendingValue));
      setState((current) => {
        if (disposed) return current;
        if (forcedReset) {
          // 新一轮开始：先归档上一轮（只要有问题和内容就归档，
// 包括被 Ctrl+Shift+A 中途打断的部分答案）。
          const previous = current.answer;
          const canArchive =
            previous.question.trim() !== "" && previous.text.trim() !== "";
          const answerHistory = canArchive
            ? [
                ...current.answerHistory,
                {
                  question: previous.question,
                  text: previous.text,
                  model: previous.model,
                },
              ].slice(-MAX_ANSWER_HISTORY)
            : current.answerHistory;
          const question =
            payloadQuestion ||
            (payload.regenerate === true
              ? previous.question
              : latestInterviewerLine(current.transcript) || previous.question);
          return {
            ...current,
            answerHistory,
            answer: {
              ...EMPTY_ANSWER,
              text,
              streaming: true,
              question,
              pending: payloadPending ?? 0,
            },
          };
        }
        // 普通增量：清掉上一轮残留的 done/error，继续追加文本。
        const reset = current.answer.done || !current.answer.streaming;
        const base = reset ? "" : current.answer.text;
        return {
          ...current,
          answer: {
            ...current.answer,
            text: base + text,
            streaming: true,
            done: false,
            error: "",
            retryable: false,
            pending: payloadPending ?? current.answer.pending,
          },
        };
      });
    });

    register(HINT_EVENTS.answerDone, (payload) => {
      if (!isRecord(payload)) return;
      const finalText = pickText(payload, ["text", "answer"]);
      const model = pickText(payload, ["model"]);
      const error = pickText(payload, ["error"]);
      const retryable = payload.retryable === true;
      const ttft = numberValue(payload.ttftMs ?? payload.ttft_ms ?? payload.ttft);
      // 错误优先：即使 text/model 为空，也不能当成「清空答案」。
      if (error !== "") {
        setState((current) => {
          if (disposed) return current;
          return {
            ...current,
            answer: {
              text: finalText !== "" ? finalText : current.answer.text,
              streaming: false,
              done: true,
              model: model || current.answer.model,
              ttftMs: ttft ?? current.answer.ttftMs,
              question: current.answer.question,
              pending: current.answer.pending,
              error,
              retryable,
            },
          };
        });
        return;
      }
      // 后端结束会话 / 判定失败时会发空 text + 空 model，视为清空答案。
      const isClear = finalText === "" && model === "";
      setState((current) => {
        if (disposed) return current;
        if (isClear) {
          return { ...current, answer: { ...EMPTY_ANSWER, done: true } };
        }
        return {
          ...current,
          answer: {
            text: finalText !== "" ? finalText : current.answer.text,
            streaming: false,
            done: true,
            model: model || current.answer.model,
            ttftMs: ttft ?? current.answer.ttftMs,
            question: current.answer.question,
            pending: current.answer.pending,
            error: "",
            retryable: false,
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

    // 排队数在入队/出队时实时变化；不要等下一轮 reset 才刷新。
    register(HINT_EVENTS.queue, (payload) => {
      if (!isRecord(payload)) return;
      const pending = numberValue(payload.pending);
      if (pending === null) return;
      const next = Math.max(0, Math.trunc(pending));
      setState((current) => {
        if (disposed) return current;
        return { ...current, answer: { ...current.answer, pending: next } };
      });
    });

    register(HINT_EVENTS.status, (payload) => {
      if (!isRecord(payload)) return;
      const nextState = pickText(payload, ["state"]) || "unknown";
      // 只在会话边界清空转录：idle（结束）/ connecting（开始新会话）。
      // listening 会被 answer 成功后再发一次，不能作为清空依据。
      const clearTranscript = nextState === "idle" || nextState === "connecting";
      const resetAnswer = nextState === "idle" || nextState === "connecting";
      setState((current) => {
        if (disposed) return current;
        return {
          ...current,
          status: {
            state: nextState,
            message: pickText(payload, ["message"]),
          },
          ...(clearTranscript
            ? { transcript: [], answerHistory: [] }
            : {}),
          ...(resetAnswer ? { answer: { ...EMPTY_ANSWER } } : {}),
        };
      });
    });

    register(HINT_EVENTS.notice, (payload) => {
      if (!isRecord(payload)) return;
      const message = pickText(payload, ["message"]);
      if (message) {
        patch({
          notice: {
            level: pickText(payload, ["level"]) || "info",
            message,
          },
        });
      }
    });

    register(HINT_EVENTS.overlayOpacity, (payload) => {
      if (!isRecord(payload)) return;
      const opacity = numberValue(payload.opacity ?? payload.value);
      if (opacity === null) return;
      applyOverlayOpacity(normalizeOpacity(opacity));
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
      answer: {
        ...EMPTY_ANSWER,
        streaming: true,
        question: current.answer.question,
        pending: current.answer.pending,
      },
    }));
    return null;
  }, []);

  const clearAnswer = useCallback(() => {
    setState((current) => ({
      ...current,
      answer: { ...EMPTY_ANSWER },
    }));
  }, []);

  /** 只关掉错误提示，保留已生成的部分答案。 */
  const dismissAnswerError = useCallback(() => {
    setState((current) => ({
      ...current,
      answer: { ...current.answer, error: "", retryable: false },
    }));
  }, []);

  return { ...state, hideOverlay, regenerate, clearAnswer, dismissAnswerError };
}

/** 最新的面试官转录行，作为 reset 帧缺少 question 时的兜底。 */
function latestInterviewerLine(transcript: TranscriptLine[]): string {
  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    const line = transcript[index];
    if (line.channel === "interviewer" && !line.isEcho) return line.text;
  }
  return "";
}