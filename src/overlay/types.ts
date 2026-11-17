export type Channel = "interviewer" | "me";

export type TranscriptLine = {
  id: number;
  channel: Channel;
  text: string;
  isEcho: boolean;
};

export type AnswerState = {
  text: string;
  streaming: boolean;
  done: boolean;
  model: string;
  ttftMs: number | null;
};

export type LevelsState = {
  mic: number;
  system: number;
};

export type StatusState = {
  state: string;
  message: string;
};

/** 后端 hint://status 会发的状态值。 */
export type SessionState =
  | "idle"
  | "connecting"
  | "listening"
  | "ready"
  | "connected"
  | "error";

export const MAX_TRANSCRIPT_LINES = 6;

/** 与 integrator 对齐的事件契约。 */
export const HINT_EVENTS = {
  transcript: "hint://transcript",
  answerDelta: "hint://answer-delta",
  answerDone: "hint://answer-done",
  levels: "hint://levels",
  status: "hint://status",
  /** 与面试会话无关的提示（模型下载、热键注册、截图预留），不覆盖会话状态。 */
  notice: "hint://notice",
} as const;