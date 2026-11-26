import { describe, expect, it } from "vitest";

/**
 * 复刻 ProfileCard 里的 organizedLabel 判据。
 *
 * 为什么单测这个纯函数：`organizedAt` 的语义有两处容易被写错——
 * 1. **0 表示「未知」而不是「1970 年」**（旧档案没有该字段时前端归一成 0）；
 * 2. 未来的时间戳（时钟回拨 / 手改配置）不该显示成「-1 天前」。
 */
function organizedLabel(organizedAt: number, now = Date.now()): string {
  if (!Number.isFinite(organizedAt) || organizedAt <= 0) return "";
  const when = new Date(organizedAt);
  if (Number.isNaN(when.getTime())) return "";
  const days = Math.floor((now - organizedAt) / 86_400_000);
  const stamp = `${when.getFullYear()}-${String(when.getMonth() + 1).padStart(2, "0")}-${String(when.getDate()).padStart(2, "0")}`;
  if (days <= 0) return `上次整理于今天（${stamp}）`;
  if (days === 1) return `上次整理于昨天（${stamp}）`;
  return `上次整理于 ${days} 天前（${stamp}）`;
}

const DAY = 86_400_000;

describe("organizedLabel —— 档案整理时间", () => {
  it("0 表示未知，返回空串（不是「1970 年」）", () => {
    // 旧档案没有 organizedAt 字段时 normalizeProfile 会填 0。
    // 显示「整理于 1970 年」会让用户以为档案坏了。
    expect(organizedLabel(0)).toBe("");
  });

  it("负数 / NaN / Infinity 都当作未知", () => {
    expect(organizedLabel(-1)).toBe("");
    expect(organizedLabel(NaN)).toBe("");
    expect(organizedLabel(Infinity)).toBe("");
  });

  it("今天整理显示「今天」", () => {
    const now = new Date(2026, 10, 27, 15, 0, 0).getTime();
    const text = organizedLabel(now - 3600_000, now);
    expect(text).toContain("今天");
    expect(text).toContain("2026-11-27");
  });

  it("昨天整理显示「昨天」", () => {
    const now = new Date(2026, 10, 27, 15, 0, 0).getTime();
    expect(organizedLabel(now - DAY, now)).toContain("昨天");
  });

  it("多天前显示天数 + 日期", () => {
    const now = new Date(2026, 10, 27, 15, 0, 0).getTime();
    const text = organizedLabel(now - 8 * DAY, now);
    expect(text).toContain("8 天前");
    expect(text).toContain("2026-11-19");
  });

  it("未来时间戳不显示成负数天（时钟回拨也不该出怪文案）", () => {
    const now = new Date(2026, 10, 27, 15, 0, 0).getTime();
    const text = organizedLabel(now + 3 * DAY, now);
    // days <= 0 走「今天」分支，不出现负号
    expect(text).not.toMatch(/-\d+ 天前/);
    expect(text).toContain("今天");
  });
});