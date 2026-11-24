import { describe, expect, it } from "vitest";
// 用 Vite 的 ?raw 直接读文本：测试环境没有 node 类型，import fs 会让 tsc 报错。
import README from "../../README.md?raw";
import {
  DEFAULT_OVERLAY_FONT_SCALE,
  DEFAULT_OVERLAY_OPACITY,
  MAX_OVERLAY_FONT_SCALE,
  MAX_OVERLAY_OPACITY,
  MIN_OVERLAY_FONT_SCALE,
  MIN_OVERLAY_OPACITY,
} from "./types";

/**
 * README 与实现的漂移守卫。
 *
 * 为什么要这条：README 曾长期写着「不做屏幕共享隐身」，而代码里
 * `overlay_content_protected` 已经实现了捕获排除；也漏写了字号、代理两个
 * 已发布功能。文档与实现分头演进、谁也不会因此变红，只能靠人记得同步。
 *
 * 这里只校验**能机器判定的那部分**：代码里定义的范围/默认值，README 里写的
 * 数字必须与之一致。文案表述无法机器判定，不在本守卫范围内。
 */
const pct = (value: number) => `${Math.round(value * 100)}%`;

describe("README 与设置项的一致性", () => {
  it("浮层不透明度的范围与默认值与代码一致", () => {
    // 代码里的权威值：0.2–1.0，默认 0.88
    const expectedRange = `${pct(MIN_OVERLAY_OPACITY)}–${pct(MAX_OVERLAY_OPACITY)}`;
    expect(README).toContain(expectedRange);
    expect(README).toContain(`默认 ${pct(DEFAULT_OVERLAY_OPACITY)}`);
  });

  it("浮层字号的范围与默认值与代码一致", () => {
    // 代码里的权威值：0.8–3.0，默认 1.0
    const expectedRange = `${pct(MIN_OVERLAY_FONT_SCALE)}–${pct(MAX_OVERLAY_FONT_SCALE)}`;
    expect(README).toContain(expectedRange);
    expect(README).toContain(`默认 ${pct(DEFAULT_OVERLAY_FONT_SCALE)}`);
  });

  it("README 不再声称「不做屏幕共享隐身」（该功能已实现，表述必须如实）", () => {
    // 这条防的是定位漂移：实现里有 overlay_content_protected，
    // README 却否认存在，会让用户根本不知道有这个开关。
    expect(README).not.toContain("不做屏幕共享隐身");
    expect(README).not.toContain("不实现「对屏幕共享隐身」");
    // 必须能搜到那个开关的说明章节
    expect(README).toContain("屏幕捕获排除");
  });

  it("README 把捕获排除的局限写出来了，没有吹成「隐身」", () => {
    // 实测：PrintWindow 路径下会变成黑块、旧式 PrintWindow 完全绕过。
    // 只写「对方看不到」而不写这两条，就是在给用户一个会在面试现场翻车的预期。
    expect(README).toContain("黑块");
    expect(README).toContain("完全绕过");
    // 必须给出更可靠的替代方案
    expect(README).toContain("Ctrl+Shift+H");
  });
});
