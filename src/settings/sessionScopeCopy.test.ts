import { describe, expect, it } from "vitest";

/**
 * 产品口径守卫：设置页必须说明「会话进行中保存的配置下次开面试才生效」。
 *
 * ## 为什么需要
 *
 * 真机验证过：会话运行中把 model 从 `deepseek-v4.1-flash` 改成
 * `deepseek-v4.1-flash-1017` 并保存，紧接着提问，落库的 `answers.model_id`
 * **仍是旧名**——会话启动时把配置取了一份快照。
 *
 * 这个行为本身是对的（避免面试中途接口突变、答案风格前后不一致），但界面上
 * 原来只有一句「配置已保存」，用户会以为新模型已经生效。
 *
 * 而同一份设置里，浮层不透明度 / 字号 / 主题是**即时生效**的（走事件下发）。
 * 两类字段行为不同却不说明，是最容易让人误判的形态。
 *
 * ## 为什么用 import.meta.glob 而不是 fs
 *
 * 前端侧 tsconfig 不带 Node 类型（`fs` / `__dirname` 都会让 `tsc --noEmit` 报错）。
 * 这个坑在 noInterviewerLabel.test.ts 里已经踩过一次。
 */
const sources = import.meta.glob("../Dashboard.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const dashboard = Object.values(sources)[0] ?? "";

/** 把源码里的换行与缩进压成单空格——JSX 文案在源码里是折行的。 */
const flat = dashboard.replace(/\s+/g, " ");

describe("设置页：会话生效口径", () => {
  it("读到 Dashboard 源码", () => {
    expect(dashboard.length).toBeGreaterThan(1000);
  });

  it("说明会话中的配置要下次开始面试才生效", () => {
    expect(flat).toMatch(/下一次「开始面试」才生效/);
    expect(flat).toMatch(/快照/);
  });

  it("说明浮层不透明度 / 字号 / 主题是即时生效的例外", () => {
    // 这是唯一一组真的即时生效的配置，漏说会让人以为「所有配置都不生效」
    expect(flat).toMatch(/不透明度、字号与主题例外/);
    expect(flat).toMatch(/即时生效/);
  });

  it("说明改配置不影响当前这场（给出现在会话里的人一个准确预期）", () => {
    // 原文案是「…都不会影响 当前这场（…）」，折行已压平，中间只剩空格
    expect(flat).toMatch(/不会影响\s*当前这场/);
  });
});