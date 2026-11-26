import { describe, expect, it } from "vitest";

/**
 * Settings 字段对等守卫：后端结构里的每个字段，前端类型都必须有。
 *
 * ## 为什么需要
 *
 * 真机验证过的缺陷（1.1.53 修）：后端 `Settings` 有 `overlay_visible`，前端
 * `Settings` 类型**没有**——而保存是 `{...form}` 发过去的，缺字段就意味着
 * 后端 serde 用默认值 `true` 填上，把磁盘上的 `false` 覆盖掉。
 *
 * 后果：用户按 Ctrl+Shift+H 隐藏浮层，改任一设置点保存，隐藏意图被抹掉，
 * 下次启动浮层又弹出来。1.1.51 刚修好「热键切换不持久化」，被这里抵消。
 *
 * 这类缺陷的共同特征：**没有报错、没有日志、单测全绿**。两侧字段名对不上，
 * 运行时静默使用默认值。只能靠结构性对等检查发现。
 *
 * ## 判据
 *
 * 后端 `pub struct Settings` 的每个字段（经 serde `rename_all = "camelCase"`
 * 转换后）都必须出现在前端 `export type Settings` 里。
 *
 * `overlayVisible` 是**有意的例外**：它由热键/托盘单方面写入，前端不持有它，
 * 保存时后端保留磁盘值（见 `commands::save_settings` 的注释）。列在
 * `INTENTIONALLY_FRONTEND_ABSENT` 里并写明理由。
 */
const rustSources = import.meta.glob("../../crates/hint-core/src/settings.rs", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const tsSources = import.meta.glob("../settings/types.ts", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const rust = Object.values(rustSources)[0] ?? "";
const ts = Object.values(tsSources)[0] ?? "";

/**
 * 有意不在前端类型里的字段。
 *
 * 加新条目必须写清理由——这条规则本身就是为了防「悄悄漏掉一个字段」，
 * 一个没有理由的白名单等于把守卫关掉。
 */
const INTENTIONALLY_FRONTEND_ABSENT: Record<string, string> = {
  overlayVisible:
    "由热键/托盘单方面写入；前端不持有它，save_settings 里保留磁盘值（避免「前端读到 true、用户按热键改 false、再保存写回 true」的竞争）",
};

function backendFields(): string[] {
  const start = rust.indexOf("pub struct Settings");
  if (start < 0) return [];
  const end = rust.indexOf("\n}", start);
  const body = rust.slice(start, end);
  const camel = /#\[serde\(rename_all = "camelCase"\)\]/.test(
    rust.slice(Math.max(0, start - 200), start),
  );
  const fields: string[] = [];
  for (const line of body.split("\n")) {
    const m = line.match(/^\s+pub (\w+):/);
    if (!m) continue;
    fields.push(
      camel ? m[1].replace(/_([a-z])/g, (_, c: string) => c.toUpperCase()) : m[1],
    );
  }
  return fields;
}

function frontendFields(): string[] {
  const start = ts.indexOf("export type Settings = {");
  if (start < 0) return [];
  const end = ts.indexOf("\n};", start);
  const body = ts.slice(start, end);
  const fields: string[] = [];
  for (const line of body.split("\n")) {
    const m = line.match(/^\s+(\w+)\??:/);
    if (m) fields.push(m[1]);
  }
  return fields;
}

describe("Settings 字段对等：后端有的，前端类型也要有", () => {
  it("读到两侧定义", () => {
    // 解析失败会让后面的断言恒真——正向对照
    expect(backendFields().length).toBeGreaterThan(10);
    expect(frontendFields().length).toBeGreaterThan(10);
    expect(backendFields()).toContain("baseUrl");
  });

  it("后端每个字段都在前端类型里（或明确豁免）", () => {
    const fe = frontendFields();
    const missing = backendFields().filter(
      (f) => !fe.includes(f) && !(f in INTENTIONALLY_FRONTEND_ABSENT),
    );
    expect(
      missing,
      `这些后端字段前端类型里没有，保存时会用默认值静默覆盖磁盘值：\n${missing.join("\n")}`,
    ).toEqual([]);
  });

  it("豁免表里的字段必须真的不在前端类型里（防过期豁免）", () => {
    const fe = frontendFields();
    const stale = Object.keys(INTENTIONALLY_FRONTEND_ABSENT).filter((f) =>
      fe.includes(f),
    );
    expect(
      stale,
      `这些字段已经加进前端类型了，应从豁免表删除：\n${stale.join("\n")}`,
    ).toEqual([]);
  });

  it("守卫本身有效：能检出缺失字段（正向对照）", () => {
    const fakeBackend = ["baseUrl", "definitelyMissingFieldXyz"];
    const fe = new Set(frontendFields());
    const missing = fakeBackend.filter((f) => !fe.has(f));
    expect(missing).toEqual(["definitelyMissingFieldXyz"]);
  });
});
