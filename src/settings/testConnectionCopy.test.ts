import { describe, expect, it } from "vitest";

/**
 * 产品口径守卫：「测试连接」只自动保存 API Key，不保存地址/模型/协议。
 *
 * ## 为什么需要
 *
 * 代码事实（`useSettingsForm.testConnection`）：调 `test_llm` 时**带上当前表单**，
 * 所以能测通未保存的 baseUrl / model；成功且表单里有 Key 时，只调 `save_api_key`
 * ——**只存 Key**。
 *
 * 真机验证：用 `deepseek-v4.1-flash-1017` 调 test_llm 成功（返回里 model 就是
 * `-1017`），但磁盘配置的 model 仍是旧值 `deepseek-v4.1-flash`。
 *
 * 原本文案写「点「测试连接」会自动保存」，用户改完地址和模型、点测试、看到
 * 「连接成功」，会以为全部存好了——重启后回到旧值。
 *
 * 用 `import.meta.glob` 读源码（前端 tsconfig 不带 Node 类型）。
 */
const sources = import.meta.glob("../settings/ModelCard.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const card = Object.values(sources)[0] ?? "";
/** JSX 文案在源码里是折行的，先压平再匹配。 */
const flat = card.replace(/\s+/g, " ");

describe("模型卡片：测试连接的保存范围", () => {
  it("读到 ModelCard 源码", () => {
    expect(card.length).toBeGreaterThan(1000);
  });

  it("说明测试连接会用当前表单（未保存也能测）", () => {
    expect(flat).toMatch(/会用\s*<strong>当前表单<\/strong>\s*去连/);
    expect(flat).toMatch(/没保存的地址和模型也能测/);
  });

  it("说明只自动保存 API Key", () => {
    expect(flat).toMatch(/成功后只\s*自动保存 API Key/);
  });

  it("明确地址、模型、协议仍需点保存全部配置", () => {
    expect(flat).toMatch(/地址、模型、协议仍需点「保存全部配置」/);
    // 还要说清不保存的后果
    expect(flat).toMatch(/回到旧值/);
  });

  it("不再出现「测试连接会自动保存」这种笼统说法", () => {
    // 旧文案：填好后点…一并保存；点「测试连接」会自动保存。
    expect(flat).not.toMatch(/点「测试连接」会自动保存/);
  });
});