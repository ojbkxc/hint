export type Channel = "interviewer" | "me";

export type TranscriptLine = {
  id: number;
  channel: Channel;
  text: string;
  isEcho: boolean;
  /** 断线补录的转录（非实时）。 */
  recovered?: boolean;
};

export type AnswerState = {
  text: string;
  streaming: boolean;
  done: boolean;
  model: string;
  /** 配置里当前选定的模型名（get_settings 读回）。没答过题时浮层显示它。 */
  configuredModel: string;
  ttftMs: number | null;
  /** 这一轮答案对应的面试官问题；空字符串表示未知。 */
  question: string;
  /** 后端队列里还有多少个问题等待回答。 */
  pending: number;
  /** 后端回答失败时的中文错误；空字符串表示无错误。 */
  error: string;
  /** 是否可以点「重试」恢复（false 时通常是配置问题）。 */
  retryable: boolean;
  /**
   * 用户在流式过程中点了「清空」：本轮剩余增量与最终全文都不再上屏，
   * 直到下一轮 reset 帧。否则答案会从半句处重新长出来。
   */
  muted: boolean;
};

/** 本场已归档的问答，用于浮层历史回看。 */
export type AnswerHistoryEntry = {
  /** 归档序号（从 1 起单调递增），用作渲染 key，不受数组截断影响。 */
  id: number;
  question: string;
  text: string;
  model: string;
};

export type LevelsState = {
  mic: number;
  system: number;
};

/** 双通道联合语音状态（谁在说话）。 */
export type SpeechState = {
  joint: "neither" | "interviewer" | "me" | "both";
};

export type StatusState = {
  state: string;
  message: string;
};

/** hint://notice 的提示；level 为 info 或 error。 */
export type NoticeState = {
  level: string;
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

/** 浮层保留的历史问答条数上限（「本场已答 N 题」用独立计数器，不受此上限影响）。 */
export const MAX_ANSWER_HISTORY = 10;

/** 与 integrator 对齐的事件契约。 */
export const HINT_EVENTS = {
  transcript: "hint://transcript",
  answerDelta: "hint://answer-delta",
  answerDone: "hint://answer-done",
  levels: "hint://levels",
  status: "hint://status",
  /** 与面试会话无关的提示（模型下载、热键注册、截图预留），不覆盖会话状态。 */
  notice: "hint://notice",
  /** 排队问题数量的实时变化（入队/出队/清空）。 */
  queue: "hint://queue",
  /** 浮层背景不透明度变化，payload { opacity: number }。 */
  overlayOpacity: "hint://overlay-opacity",
  /** 浮层字号缩放变化，payload { scale: number }（0.8–1.6，1.0 = 默认）。 */
  overlayFontScale: "hint://overlay-font-scale",
  /** 主题变化（设置页保存后实时生效），payload { theme: "light" | "dark" }。 */
  overlayTheme: "hint://overlay-theme",
  /** 双通道联合语音状态迁移，payload { channel, speaking, joint, atMs, msSinceOtherEdge }。 */
  speechEdge: "hint://speech-edge",
} as const;