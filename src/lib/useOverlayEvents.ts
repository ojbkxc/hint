import { useCallback, useEffect, useRef, useState } from "react";
import {
  HINT_EVENTS,
  MAX_ANSWER_HISTORY,
  MAX_TRANSCRIPT_LINES,
} from "../overlay/types";
import type {
  AnswerHistoryEntry,
  AnswerState,
  LevelsState,
  NoticeState,
  SpeechState,
  StatusState,
  TranscriptLine,
} from "../overlay/types";
import { isRecord, numberValue, pickText } from "./data";
import { safeInvoke, safeListen } from "./tauri";
import { normalizeOpacity, normalizeSettings } from "../settings/types";

type OverlayListenerState = {
  transcript: TranscriptLine[];
  answer: AnswerState;
  /** 本场已回答过的问答（不含当前这一轮），按时间先后追加。 */
  answerHistory: AnswerHistoryEntry[];
  /** 本场已归档的问答总数；answerHistory 会被截断，计数不能从它推。 */
  answeredTotal: number;
  levels: LevelsState;
  /** 双通道联合语音状态（谁在说话）。 */
  speech: SpeechState;
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
  muted: false,
};

const INITIAL_STATE: OverlayListenerState = {
  transcript: [],
  answer: EMPTY_ANSWER,
  answerHistory: [],
  answeredTotal: 0,
  levels: { mic: 0, system: 0 },
  speech: { joint: "neither" },
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
        recovered: payload.recovered === true,
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
          // 序号取「上一条 + 1」，不用自增 ref：setState 更新函数必须保持纯，
          // 否则 StrictMode 双调用会给出不同的 key。
          const last = current.answerHistory[current.answerHistory.length - 1];
          const answerHistory = canArchive
            ? [
                ...current.answerHistory,
                {
                  id: (last?.id ?? 0) + 1,
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
            answeredTotal: canArchive
              ? current.answeredTotal + 1
              : current.answeredTotal,
            answer: {
              ...EMPTY_ANSWER,
              text,
              streaming: true,
              question,
              pending: payloadPending ?? 0,
            },
          };
        }
        // 用户点了「清空」但本轮还在流式：剩余增量不上屏，只更新排队数。
        if (current.answer.muted) {
          return {
            ...current,
            answer: {
              ...current.answer,
              pending: payloadPending ?? current.answer.pending,
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
              // 用户已经清空过这一轮：错误条照样提示，但不要把正文贴回来。
              text: current.answer.muted
                ? ""
                : (finalText !== "" ? finalText : current.answer.text),
              streaming: false,
              done: true,
              model: model || current.answer.model,
              ttftMs: ttft ?? current.answer.ttftMs,
              question: current.answer.question,
              pending: current.answer.pending,
              error,
              retryable,
              muted: current.answer.muted,
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
            // 流式中被「清空」过：最终全文不再上屏，避免答案整段复活。
            text: current.answer.muted
              ? ""
              : (finalText !== "" ? finalText : current.answer.text),
            streaming: false,
            done: true,
            model: model || current.answer.model,
            ttftMs: ttft ?? current.answer.ttftMs,
            question: current.answer.question,
            pending: current.answer.pending,
            error: "",
            retryable: false,
            muted: current.answer.muted,
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

    // 双通道联合语音状态：谁在说话（面试官 / 我 / 同时 / 无）。
    register(HINT_EVENTS.speechEdge, (payload) => {
      if (!isRecord(payload)) return;
      const joint = payload.joint;
      if (
        joint === "neither" ||
        joint === "interviewer" ||
        joint === "me" ||
        joint === "both"
      ) {
        patch({ speech: { joint } });
      }
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
            ? { transcript: [], answerHistory: [], answeredTotal: 0 }
            : {}),
          ...(resetAnswer
            ? { answer: { ...EMPTY_ANSWER }, speech: { joint: "neither" } }
            : {}),
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
    // 不要在这里清空答案文本：后端随后会发 reset 帧，并在那时把上一轮答案
    // 归档到时间线。若前端提前清空，reset 帧看到 previous.text 为空就不会
    // 归档，旧答案会从「更早的回答」里消失。这里只切到流式状态，等 reset
    // 帧统一清空并归档。
    setState((current) => ({
      ...current,
      answer: {
        ...current.answer,
        streaming: true,
        done: false,
        error: "",
        retryable: false,
      },
    }));
    return null;
  }, []);

  const clearAnswer = useCallback(() => {
    setState((current) => ({
      ...current,
      // 「清空」只清浮层显示；后端的待答队列还在，排队数不能被抹成 0。
      answer: {
        ...EMPTY_ANSWER,
        pending: current.answer.pending,
        // 正在流式时清空：本轮剩余增量与最终全文都不再上屏，
        // 否则下一帧增量会从半句处重新长出来。
        muted: current.answer.streaming,
      },
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