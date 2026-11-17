/** 后端返回值形状不确定时的防御式读取工具。 */

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function textValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return "";
}

export function numberValue(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

/** 依次尝试多个字段名，返回第一个可读的文本值。 */
export function pickText(obj: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const text = textValue(obj[key]);
    if (text !== "") return text;
  }
  return "";
}

export function pickNumber(obj: Record<string, unknown>, keys: string[]): number | null {
  for (const key of keys) {
    const parsed = numberValue(obj[key]);
    if (parsed !== null) return parsed;
  }
  return null;
}