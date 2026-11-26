import { describe, expect, it } from "vitest";

/**
 * 产品口径守卫：界面上不出现「面试官」这个词。
 *
 * ## 为什么需要一条结构测试
 *
 * 改成「提问 / 对方」这件事做过两轮，**两轮都漏了地方**：
 *
 * 1. 第一轮漏了复盘页图例、设备卡片、资料卡片标题（本轮才发现）；
 * 2. 每轮都是靠肉眼在截图里看到才发现的——界面有十几个区块，漏一处就要等用户撞上。
 *
 * 靠「改的时候记得全」不可靠；这里用源码扫描把口径钉住。
 *
 * ## 为什么用 import.meta.glob 而不是 fs
 *
 * 前端侧 tsconfig 不带 Node 类型（`fs` / `__dirname` 都会让 `tsc --noEmit` 报错）。
 * Vite 的 `import.meta.glob` 在测试环境（vite 驱动）里可用，且是类型安全的，
 * 不需要额外引入 `@types/node` 或改 tsconfig。
 */
const sources = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** 只保留真正的源文件：排除测试自身与 node_modules。 */
function sourceEntries(): Array<[string, string]> {
  return Object.entries(sources).filter(
    ([file]) => !/\.test\.(ts|tsx)$/.test(file) && !file.includes("node_modules"),
  );
}

/**
 * 判断某一行是否为「纯注释」——注释里的「面试官」是给开发者看的，不违规。
 *
 * 只认行首的 `//`、`*`、`/*`：行尾注释（代码 + `// …`）里出现该词仍算违规，
 * 因为那种写法一旦被误改成字符串就没人注意。
 */
function isCommentLine(line: string): boolean {
  const trimmed = line.trimStart();
  return (
    trimmed.startsWith("//") ||
    trimmed.startsWith("*") ||
    trimmed.startsWith("/*") ||
    trimmed.startsWith("/**")
  );
}

/** 收集所有违规：返回 `文件:行: 内容` 形式，便于直接定位。 */
function violationsIn(entries: Array<[string, string]>): string[] {
  const out: string[] = [];
  for (const [file, content] of entries) {
    const rel = file.replace(/^\.\.\//, "");
    content.split("\n").forEach((line, index) => {
      if (!line.includes("面试官")) return;
      if (isCommentLine(line)) return;
      out.push(`${rel}:${index + 1}: ${line.trim().slice(0, 70)}`);
    });
  }
  return out;
}

describe("产品口径：界面文案不出现「面试官」", () => {
  it("扫描范围覆盖 src 全目录（防「扫错目录→假绿」）", () => {
    // 第一版用 `path.resolve(__dirname)` 把范围写成了 `src/lib`，只扫到 5 个文件，
    // 关键界面文件全在范围外、守卫永远绿。这条断言把那个坑钉死：
    // 一旦 glob 的相对路径写错（比如写成 `./**`），文件数会立刻掉下来。
    const files = sourceEntries().map(([f]) => f.replace(/^\.\.\//, ""));
    expect(files.length, "扫到的文件太少，glob 路径可能写错了").toBeGreaterThan(20);
    for (const required of [
      "sessions/SessionReview.tsx",
      "settings/DevicesCard.tsx",
      "settings/ProfileCard.tsx",
      "overlay/TranscriptPanel.tsx",
      "Overlay.tsx",
    ]) {
      expect(files, `扫描范围必须包含 ${required}`).toContain(required);
    }
  });

  it("源码里没有用户可见的「面试官」", () => {
    const violations = violationsIn(sourceEntries());
    expect(
      violations,
      `这些位置仍会显示「面试官」，应改为「提问」（标签/图例）或「对方」（描述声音来源）：\n${violations.join("\n")}`,
    ).toEqual([]);
  });

  it("守卫本身有效：能检出违规（正向对照）", () => {
    // 如果扫描逻辑写错（比如永远返回空），上面那条会假绿——用合成样本验证。
    const synthetic: Array<[string, string]> = [
      [
        "fake/Sample.tsx",
        [
          'const a = "面试官";',
          "// 面试官（注释，允许）",
          "  * 面试官（块注释续行，允许）",
          'const b = "提问";',
        ].join("\n"),
      ],
    ];
    const violations = violationsIn(synthetic);
    expect(violations).toHaveLength(1);
    expect(violations[0]).toContain('const a = "面试官"');
  });
});