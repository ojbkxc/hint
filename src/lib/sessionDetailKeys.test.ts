import { describe, expect, it } from "vitest";

/**
 * 会话复盘：`session_detail` 返回的键必须覆盖前端类型声明的键。
 *
 * ## 为什么需要
 *
 * 真机验证过的缺陷：后端 `Segment` 结构有 `recovered`，但 `session_detail` 命令
 * 的 `json!` 里**漏了这个键**。前端 `SessionSegment` 声明了 `recovered: boolean`，
 * 复盘页据此显示「补录」标记——字段永远是 `undefined` → `record.recovered === true`
 * 是 false → 标记永远不显示。
 *
 * 具体后果：会话 36 有 **21 条补录段落**，在复盘页上和实时转录长得一模一样，
 * 用户没法区分哪些是断线后补的。
 *
 * 这类缺陷（命令层手工 json! 与前端类型漂移）和 1.1.46 的 `Profile` 缺 camelCase、
 * 1.1.53 的 `overlay_visible` 是同一形态：**没有报错、没有日志、单测全绿**。
 *
 * ## 判据
 *
 * `commands.rs` 里 `session_detail` 构造的 segment / answer / 顶层 `json!`，
 * 其键名必须覆盖前端 `SessionSegment` / `SessionAnswer` / `SessionDetail` 声明的键。
 *
 * 反向（后端多返回的键，如 `totalMs`）不判——多余字段无害，缺失才有害。
 */
const cmdSources = import.meta.glob("../../src-tauri/src/commands.rs", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const typeSources = import.meta.glob("../sessions/types.ts", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const cmd = Object.values(cmdSources)[0] ?? "";
const types = Object.values(typeSources)[0] ?? "";

/** 取前端某个 type 声明的键。 */
function typeKeys(typeName: string): string[] {
  const start = types.indexOf(`export type ${typeName} = {`);
  if (start < 0) return [];
  const end = types.indexOf("\n};", start);
  const body = types.slice(start, end);
  const keys: string[] = [];
  for (const line of body.split("\n")) {
    const m = line.match(/^\s+(\w+)\??:/);
    if (m) keys.push(m[1]);
  }
  return keys;
}

/**
 * 从 commands.rs 里截出 `session_detail` 函数体，再取其中所有 `json!({...})`
 * 的字符串键名。
 */
function detailJsonKeys(): { segment: string[]; answer: string[]; top: string[] } {
  const start = cmd.indexOf("pub fn session_detail");
  if (start < 0) return { segment: [], answer: [], top: [] };
  // 到下一个 `#[tauri::command` 或下一个 pub fn 为止
  const rest = cmd.slice(start);
  const nextFn = rest.search(/\n#\[tauri::command\]|\npub fn /);
  const body = nextFn > 0 ? rest.slice(0, nextFn) : rest;

  // 按出现顺序：第一个 json! 是 segment，第二个是 answer，第三个是顶层
  const blocks = [...body.matchAll(/json!\(\{([\s\S]*?)\}\)/g)].map((m) => m[1]);
  const keysOf = (text: string) => {
    const out: string[] = [];
    for (const m of text.matchAll(/"(\w+)":/g)) out.push(m[1]);
    return out;
  };
  return {
    segment: blocks[0] ? keysOf(blocks[0]) : [],
    answer: blocks[1] ? keysOf(blocks[1]) : [],
    top: blocks[2] ? keysOf(blocks[2]) : [],
  };
}

const jsonKeys = detailJsonKeys();

describe("session_detail 返回键覆盖前端类型", () => {
  it("解析到两侧定义", () => {
    // 解析失败会让后面的断言恒真——正向对照
    expect(typeKeys("SessionSegment").length).toBeGreaterThan(3);
    expect(jsonKeys.segment.length).toBeGreaterThan(3);
    expect(jsonKeys.segment).toContain("text");
  });

  it("segment：前端声明的键都在后端 json! 里", () => {
    const missing = typeKeys("SessionSegment").filter(
      (k) => !jsonKeys.segment.includes(k),
    );
    expect(
      missing,
      `这些键前端 SessionSegment 声明了、但 session_detail 的 json! 没返回，\n` +
        `运行时永远是 undefined（复盘页相关展示会静默失效）：\n${missing.join("\n")}`,
    ).toEqual([]);
  });

  it("answer：前端声明的键都在后端 json! 里", () => {
    const missing = typeKeys("SessionAnswer").filter(
      (k) => !jsonKeys.answer.includes(k),
    );
    expect(missing, `缺失的 answer 键：\n${missing.join("\n")}`).toEqual([]);
  });

  it("顶层：summary 的键都在后端 json! 里（后端是扁平返回）", () => {
    // 注意：不要拿 `SessionDetail` 的键来比——它的 `summary` 是**前端构造**的：
    // `normalizeSessionDetail` 用 `normalizeSummary(raw, 0)` 从同一层读 startedAt /
    // endedAt / model / note。后端顶层就是这几个字段的平铺，外加 segments / answers。
    // 第一版按 `SessionDetail` 比，误报「summary 缺失」。
    const missing = typeKeys("SessionSummary").filter(
      (k) => !jsonKeys.top.includes(k),
    );
    expect(missing, `缺失的 summary 键：\n${missing.join("\n")}`).toEqual([]);
    // 数组字段也要在
    for (const arrayKey of ["segments", "answers"]) {
      expect(jsonKeys.top, `顶层缺 ${arrayKey}`).toContain(arrayKey);
    }
  });

  it("守卫本身有效：缺键会被检出（正向对照）", () => {
    // 模拟「前端有 recovered、后端没有」的历史形态
    const feKeys = ["id", "channel", "text", "isEcho", "recovered", "startMs"];
    const beKeys = ["id", "channel", "text", "isEcho", "startMs"];
    const missing = feKeys.filter((k) => !beKeys.includes(k));
    expect(missing).toEqual(["recovered"]);
  });
});
