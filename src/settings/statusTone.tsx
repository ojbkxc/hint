export type StatusTone = "success" | "warning" | "error" | "info";

const WARNING_MARKERS = ["但", "未完成", "没有返回可用模型"];
const SUCCESS_MARKERS = [
  "已保存",
  "已获取",
  "连接成功",
  "已导入",
  "已删除",
  "已清空",
  "已导出",
  "检查完成",
  "已启动",
  "已结束",
];

/** 在 tsx 展示层派生状态语义色，不参与任何业务判断，也不改动 hook 的单一 status。 */
export function deriveStatusTone(text: string): StatusTone {
  const t = text.trim();
  if (!t) return "info";
  if (WARNING_MARKERS.some((m) => t.includes(m))) return "warning";
  if (/失败[:：]?|错误[:：]?/.test(t)) return "error";
  if (SUCCESS_MARKERS.some((m) => t.includes(m))) return "success";
  return "info";
}

/** 语义色 → 既有反馈类名（成功绿 / 警告黄 / 错误红 / 信息蓝）。 */
export function statusToneClass(tone: StatusTone): string {
  switch (tone) {
    case "success":
      return "status";
    case "warning":
      return "status-warning";
    case "error":
      return "error-text";
    case "info":
    default:
      return "status-info";
  }
}
