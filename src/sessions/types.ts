import { isRecord, pickNumber, pickText, textValue } from "../lib/data";

export type SessionSummary = {
  id: string;
  startedAt: string;
  endedAt: string;
  model: string;
  note: string;
};

export type SessionSegment = {
  id: string;
  channel: "interviewer" | "me";
  text: string;
  isEcho: boolean;
  startMs: number | null;
};

export type SessionAnswer = {
  id: string;
  text: string;
  model: string;
  ttftMs: number | null;
};

export type SessionDetail = {
  id: string;
  summary: SessionSummary;
  segments: SessionSegment[];
  answers: SessionAnswer[];
};

/** 后端可能返回数组或 {sessions:[...]}，都兼容。 */
export function normalizeSessions(raw: unknown): SessionSummary[] {
  const list = Array.isArray(raw)
    ? raw
    : isRecord(raw) && Array.isArray(raw.sessions)
      ? raw.sessions
      : [];
  return list.map((item, index) => normalizeSummary(item, index)).filter((s) => s.id !== "");
}

function normalizeSummary(raw: unknown, index: number): SessionSummary {
  if (!isRecord(raw)) {
    return {
      id: `session-${index}`,
      startedAt: "",
      endedAt: "",
      model: "",
      note: String(raw ?? ""),
    };
  }
  const id = pickText(raw, ["id", "sessionId", "session_id"]) || `session-${index}`;
  return {
    id,
    startedAt: pickText(raw, ["startedAt", "started_at", "start"]),
    endedAt: pickText(raw, ["endedAt", "ended_at", "end"]),
    model: pickText(raw, ["model", "modelId", "model_id"]),
    note: pickText(raw, ["note", "title"]),
  };
}

export function normalizeSessionDetail(raw: unknown, id: string): SessionDetail {
  if (!isRecord(raw)) {
    return { id, summary: normalizeSummary(raw, 0), segments: [], answers: [] };
  }
  const summary = normalizeSummary(raw, 0);
  summary.id = id;

  const segmentsRaw = Array.isArray(raw.segments) ? raw.segments : [];
  const segments = segmentsRaw.map((item, index) => {
    const record = isRecord(item) ? item : {};
    return {
      id: pickText(record, ["id"]) || `segment-${index}`,
      channel: record.channel === "me" ? ("me" as const) : ("interviewer" as const),
      text: pickText(record, ["text"]),
      isEcho: record.isEcho === true || record.is_echo === true,
      startMs: pickNumber(record, ["startMs", "start_ms", "start"]),
    };
  });

  const answersRaw = Array.isArray(raw.answers) ? raw.answers : [];
  const answers = answersRaw.map((item, index) => {
    const record = isRecord(item) ? item : {};
    return {
      id: pickText(record, ["id"]) || `answer-${index}`,
      text: pickText(record, ["text", "answer"]),
      model: pickText(record, ["modelId", "model_id", "model"]),
      ttftMs: pickNumber(record, ["ttftMs", "ttft_ms", "ttft"]),
    };
  });

  return { id, summary, segments, answers };
}

/** 把导出结果（字符串或 {markdown}）统一成 Markdown 文本。 */
export function normalizeMarkdown(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (isRecord(raw)) {
    const markdown = pickText(raw, ["markdown", "content", "text"]);
    if (markdown) return markdown;
  }
  const fallback = textValue(raw);
  return fallback;
}

export function downloadMarkdown(filename: string, content: string) {
  const blob = new Blob([content], { type: "text/markdown;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}