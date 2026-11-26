import { describe, expect, it } from "vitest";

/**
 * IPC 契约守卫：前端调用的命令必须都在后端注册，且不留零调用点的死命令。
 *
 * ## 为什么需要
 *
 * 上轮（1.1.51）靠一次性脚本发现 4 个「注册了但前端从不调用」的命令，其中
 * `set_overlay_content_protected` 的存在会让人以为「内容保护靠调这个命令」，
 * 实际是保存配置时应用——这种误导刚刚在 `manifest.rs` 上坑过一次（注释描述了
 * 一个不存在的按钮）。
 *
 * 当时没把检查留下来，下次新增命令时同样的腐烂会重演。这里把它固化成守卫。
 *
 * ## 判据
 *
 * 1. **前端 invoke 的每个命令都必须在 `generate_handler!` 里**——
 *    否则用户点了就报 "Command not found"；
 * 2. 反向检查：注册了但**前端与 Rust 内部都没有调用点**的命令应当删除。
 *    这条允许例外（热键/托盘那条路径在 Rust 侧调用），所以判据是
 *    「前端无引用 **且** Rust 侧也没有 `commands::<name>(` 调用」。
 *
 * 用 `import.meta.glob` 之外的方式读文件：这里是 Node 侧测试，但前端 tsconfig
 * 不带 Node 类型，所以仍走 `import.meta.glob`。
 */
const feSources = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const libSource = Object.values(
  import.meta.glob("../../src-tauri/src/lib.rs", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>,
)[0] ?? "";

const cmdSources = Object.values(
  import.meta.glob("../../src-tauri/src/*.rs", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>,
);

/** 前端调用的命令名 → 引用它的文件集合。 */
function frontendCommands(): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>();
  const re = /(?:safeInvoke|invoke)(?:<[^>]*>)?\s*\(\s*["']([a-z_][a-z0-9_]*)["']/g;
  for (const [file, text] of Object.entries(feSources)) {
    if (/\.test\.(ts|tsx)$/.test(file)) continue;
    for (const match of text.matchAll(re)) {
      const name = match[1];
      if (!out.has(name)) out.set(name, new Set());
      out.get(name)!.add(file.replace(/^\.\.\//, ""));
    }
  }
  return out;
}

/** `generate_handler![...]` 里注册的命令名。 */
function registeredCommands(): Set<string> {
  const out = new Set<string>();
  const block = libSource.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!block) return out;
  for (const line of block[1].split("\n")) {
    const trimmed = line.trim().replace(/,$/, "");
    if (trimmed.startsWith("commands::")) {
      out.add(trimmed.slice("commands::".length));
    }
  }
  return out;
}

const fe = frontendCommands();
const registered = registeredCommands();

describe("IPC 契约：命令注册一致性", () => {
  it("读到后端注册列表", () => {
    // 解析失败会让后面的断言恒真——必须正向对照
    expect(registered.size).toBeGreaterThan(20);
    expect(registered.has("get_settings")).toBe(true);
  });

  it("前端调用的每个命令都在后端注册", () => {
    const missing = [...fe.entries()]
      .filter(([name]) => !registered.has(name))
      .map(([name, files]) => `${name}（${[...files].join(", ")}）`);
    expect(
      missing,
      `这些命令前端会调、后端没注册，用户点了会报 Command not found：\n${missing.join("\n")}`,
    ).toEqual([]);
  });

  it("不留「前端与 Rust 都不调用」的死命令", () => {
    const allRust = cmdSources.join("\n") + "\n" + libSource;
    const dead = [...registered].filter((name) => {
      if (fe.has(name)) return false;
      // Rust 内部调用：热键/托盘那条路径会写成 commands::<name>(...)
      return !allRust.includes(`commands::${name}(`);
    });
    expect(
      dead,
      `这些命令注册了但前端与 Rust 都不调用，应删除（降低 IPC 暴露面、避免误导）：\n${dead.join("\n")}`,
    ).toEqual([]);
  });

  it("守卫本身有效：能检出未注册的命令（正向对照）", () => {
    // 造一个必然不存在的命令名，验证判据不会永远返回空
    const fake = "definitely_not_a_registered_command_xyz";
    expect(registered.has(fake)).toBe(false);
    // 且真集合里确实有内容
    expect(registered.size).toBeGreaterThan(0);
  });
});
