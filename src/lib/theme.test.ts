import { beforeEach, describe, expect, it } from "vitest";
import { applyTheme } from "./theme";

/**
 * 主题落点的守卫。
 *
 * 为什么需要它：主题在 CSS 侧只是 `[data-theme="dark"]` 一段变量覆盖，
 * 一旦 `applyTheme` 写错属性名（`data-theme` vs `dataset.theme` 的大小写映射、
 * 或者忘了在切回浅色时删属性），页面会停在深色且没有任何报错——测试里
 * 唯一能区分新旧的就是「根元素上到底有没有那个属性」。
 */
describe("applyTheme", () => {
  beforeEach(() => {
    delete document.documentElement.dataset.theme;
  });

  it("dark 时在根元素写 data-theme=dark", () => {
    const applied = applyTheme("dark");
    expect(applied).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    // 属性名必须正好是 data-theme：CSS 侧按这个选择器匹配。
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("light 时把属性整个删掉（默认状态就是浅色）", () => {
    applyTheme("dark");
    const applied = applyTheme("light");
    expect(applied).toBe("light");
    expect(document.documentElement.dataset.theme).toBeUndefined();
    // 不能写成 data-theme="light"：浅色必须等于升级前的 DOM。
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("未知值 / 缺省值回落浅色，不会留下半黑状态", () => {
    applyTheme("dark");
    for (const raw of [undefined, null, "", "midnight", 42, {}]) {
      expect(applyTheme(raw)).toBe("light");
      expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
    }
  });

  it("重复应用同一主题是幂等的", () => {
    applyTheme("dark");
    applyTheme("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});
