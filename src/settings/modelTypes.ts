export type DownloadStep = "model" | "verifying" | "vad";

/**
 * 后端 ModelStatus（serde tag=phase、camelCase）：
 * - { "phase": "ready", "modelDir": "C:\\..." }
 * - { "phase": "unprepared", "downloadedBytes": 0, "totalBytes": 241357257 }
 * - { "phase": "downloading", "step": "model", "resource": "model.int8.onnx",
 *     "completedBytes": 123, "totalBytes": 241357257 }
 */
export type ModelPhase = "loading" | "ready" | "unprepared" | "downloading" | "error";

export type ModelView = {
  phase: ModelPhase;
  modelDir: string;
  /** 未下载时已落盘的字节数。 */
  downloadedBytes: number;
  /** 当前任务已完成字节数。 */
  completedBytes: number;
  totalBytes: number;
  step: DownloadStep;
  resource: string;
  source: string;
  error: string;
};

export const DEFAULT_TOTAL_BYTES = 241_357_257;

export const INITIAL_MODEL_VIEW: ModelView = {
  phase: "loading",
  modelDir: "",
  downloadedBytes: 0,
  completedBytes: 0,
  totalBytes: DEFAULT_TOTAL_BYTES,
  step: "model",
  resource: "",
  source: "",
  error: "",
};

function numOf(value: Record<string, unknown>, key: string, fallback: number): number {
  const raw = value[key];
  const parsed = typeof raw === "number" ? raw : Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function strOf(value: Record<string, unknown>, key: string): string {
  return typeof value[key] === "string" ? (value[key] as string) : "";
}

/** model_status 返回值容错：未知/缺失 phase 一律按「未就绪」处理，允许用户下载。 */
export function normalizeModelStatus(raw: unknown): ModelView {
  if (typeof raw !== "object" || raw === null) {
    return { ...INITIAL_MODEL_VIEW, phase: "unprepared" };
  }
  const value = raw as Record<string, unknown>;
  const totalBytes = numOf(value, "totalBytes", DEFAULT_TOTAL_BYTES);

  switch (value.phase) {
    case "ready":
      return { ...INITIAL_MODEL_VIEW, phase: "ready", modelDir: strOf(value, "modelDir"), totalBytes };
    case "downloading": {
      const completedBytes = numOf(value, "completedBytes", 0);
      return {
        ...INITIAL_MODEL_VIEW,
        phase: "downloading",
        step: (strOf(value, "step") as DownloadStep) || "model",
        resource: strOf(value, "resource"),
        completedBytes,
        downloadedBytes: completedBytes,
        totalBytes,
      };
    }
    case "unprepared": {
      const downloadedBytes = numOf(value, "downloadedBytes", 0);
      return {
        ...INITIAL_MODEL_VIEW,
        phase: "unprepared",
        downloadedBytes,
        completedBytes: downloadedBytes,
        totalBytes,
      };
    }
    default:
      return { ...INITIAL_MODEL_VIEW, phase: "unprepared", totalBytes };
  }
}

/** 进度事件 payload：{step, resource, completedBytes, totalBytes, source}。 */
export function applyProgress(view: ModelView, payload: unknown): ModelView {
  if (typeof payload !== "object" || payload === null) return view;
  const value = payload as Record<string, unknown>;
  const completedBytes = numOf(value, "completedBytes", view.completedBytes);
  return {
    ...view,
    phase: "downloading",
    error: "",
    step: (strOf(value, "step") as DownloadStep) || view.step,
    resource: strOf(value, "resource") || view.resource,
    source: strOf(value, "source") || view.source,
    completedBytes,
    downloadedBytes: completedBytes,
    totalBytes: numOf(value, "totalBytes", view.totalBytes),
  };
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${(bytes / 1024).toFixed(0)} KB`;
  if (mb < 1000) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

export function percentOf(completed: number, total: number): number {
  if (!Number.isFinite(total) || total <= 0) return 0;
  return Math.min(100, Math.max(0, (completed / total) * 100));
}

export function stepLabel(step: DownloadStep): string {
  const map: Record<string, string> = {
    model: "下载模型",
    verifying: "校验文件",
    vad: "下载 VAD",
  };
  return map[step] ?? step;
}

export function sourceLabel(source: string): string {
  if (source === "official") return "官方源";
  if (source === "mirror") return "镜像源";
  return source;
}