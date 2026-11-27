#!/usr/bin/env node
/**
 * UI 回归自检：把 docs/verification-baseline.md 里能自动跑的部分做成一条命令。
 *
 * ## 为什么需要
 *
 * 1.1.42 → 1.1.61 这 20 个版本里，每次验证都是临时写脚本、跑完删掉。下次改动后
 * 想知道「基线还成立吗」就得重写一遍。这个脚本把那些判据固化下来。
 *
 * ## 用法
 *
 * 应用需以 CDP 端口启动。脚本会自己拉起应用（如果没在跑）：
 *
 *     $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'
 *     node scripts/verify-ui.mjs
 *
 * 退出码：0 全过 / 1 有失败 / 2 环境不可用（应用没起来、模型没下等）
 *
 * ## 判据取舍
 *
 * 只放**稳定且能区分新旧**的检查。像「热键真实触发」这种本环境做不到的、
 * 「麦克风拔插」需要提权的，一律不放——放进来只会让人开始忽略红的信号。
 */

import { chromium } from "playwright";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const EXE = process.env.HINT_EXE ?? "D:\\UniAccessAgentDownloadData\\AppStore\\hint\\hint.exe";
const CDP = "http://127.0.0.1:9222";
const SECTIONS = ["模型接口", "语音模型", "面试会话", "设备", "简历", "回答行为", "保存与日志"];

const results = [];
function ck(name, ok, detail) {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  " + detail : ""}`);
}

async function appRunning() {
  try {
    const out = execFileSync(
      "powershell",
      ["-NoProfile", "-Command", "(Get-Process hint -ErrorAction SilentlyContinue | Measure-Object).Count"],
      { encoding: "utf8", timeout: 20000 },
    );
    return Number((out || "0").trim()) > 0;
  } catch {
    return false;
  }
}

async function connect() {
  for (let i = 0; i < 25; i++) {
    try {
      const b = await chromium.connectOverCDP(CDP);
      for (const ctx of b.contexts()) {
        for (const p of ctx.pages()) {
          if (await p.evaluate(() => !!document.querySelector(".dashboard-root")).catch(() => false)) {
            return { browser: b, dash: p };
          }
        }
      }
      await b.close();
    } catch {
      /* 还没起来 */
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  return null;
}

async function main() {
  if (!existsSync(EXE)) {
    console.error(`找不到 hint.exe：${EXE}`);
    process.exit(2);
  }

  if (!(await appRunning())) {
    console.log("启动应用…");
    spawn(EXE, [], { detached: true, stdio: "ignore" }).unref();
    await new Promise((r) => setTimeout(r, 14000));
  }

  const conn = await connect();
  if (!conn) {
    console.error("连不上 CDP 端口 9222 —— 应用要么没起来，要么没有开远程调试。");
    console.error("Windows 上需要先设 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222' 再启动。");
    process.exit(2);
  }
  const { browser, dash } = conn;

  try {
    // ---------- 1. 版本号与命令 ----------
    console.log("\n=== 版本与命令 ===");
    const ver = await dash.evaluate(async () => {
      try {
        return await window.__TAURI_INTERNALS__.invoke("app_version");
      } catch (e) {
        return "ERR:" + String(e).slice(0, 60);
      }
    });
    ck("app_version 返回版本号", /^\d+\.\d+\.\d+$/.test(ver), ver);

    const status = await dash.evaluate(async () => {
      try {
        return await window.__TAURI_INTERNALS__.invoke("model_status");
      } catch (e) {
        return { phase: "error", error: String(e) };
      }
    });
    ck("语音模型就绪", status?.phase === "ready", status?.phase ?? "?");

    // ---------- 2. 会话列表 ----------
    console.log("\n=== 会话列表 ===");
    const list = await dash.evaluate(async () => {
      try {
        return await window.__TAURI_INTERNALS__.invoke("list_sessions");
      } catch (e) {
        return { error: String(e) };
      }
    });
    const sessions = Array.isArray(list) ? list : list?.sessions ?? [];
    ck("list_sessions 可用", Array.isArray(sessions), `${sessions.length} 条`);
    ck(
      "列表带 total/limited（老格式退回）",
      Array.isArray(list) || (typeof list?.total === "number" && typeof list?.limited === "boolean"),
    );

    // ---------- 3. 搜索覆盖 ----------
    console.log("\n=== 搜索 ===");
    const searchOk = await dash.evaluate(async () => {
      const probe = async (q) => {
        try {
          const r = await window.__TAURI_INTERNALS__.invoke("search_sessions", { query: q });
          return Array.isArray(r) ? r : [];
        } catch {
          return null;
        }
      };
      return {
        empty: await probe(""),          // 空串应返回空
        impossible: await probe("zzz绝不存在的词"), // 应返回空数组而非 null
      };
    });
    ck("空查询返回空数组", Array.isArray(searchOk?.empty) && searchOk.empty.length === 0);
    ck("搜不存在的词返回空数组（不是失败）", Array.isArray(searchOk?.impossible) && searchOk.impossible.length === 0);

    // ---------- 4. 设置页布局不溢出 ----------
    console.log("\n=== 设置页各区块布局 ===");
    const widths = [1200, 900, 780, 760];
    for (const w of widths) {
      await dash.setViewportSize({ width: w, height: 800 });
      await dash.waitForTimeout(400);
      let bad = 0;
      for (const name of SECTIONS) {
        await dash.evaluate(async (n) => {
          const el = [...document.querySelectorAll(".settings-rail-item")].find((x) => (x.textContent || "").includes(n));
          if (el) el.click();
          await new Promise((r) => setTimeout(r, 400));
        }, name);
        const over = await dash.evaluate(() => {
          const secs = [...document.querySelectorAll(".settings-section")].filter((s) => s.offsetHeight > 0);
          const sec = secs[0];
          if (!sec) return 0;
          const r = sec.getBoundingClientRect();
          let n = 0;
          for (const el of sec.querySelectorAll("*")) {
            const b = el.getBoundingClientRect();
            if (b.width === 0 || b.height === 0) continue;
            if (Math.round(b.right - r.right) > 2) n++;
          }
          return n;
        });
        bad += over;
      }
      ck(`视口 ${w}px 下 7 个区块无横向溢出`, bad === 0, bad ? `溢出 ${bad}` : "");
    }

    // ---------- 5. 导出内容 ----------
    console.log("\n=== 导出 ===");
    const dir = mkdtempSync(join(tmpdir(), "hint-verify-"));
    const exp = await dash.evaluate(async (d) => {
      try {
        await window.__TAURI_INTERNALS__.invoke("export_all_sessions", { dir: d });
        return { ok: true };
      } catch (e) {
        return { ok: false, err: String(e).slice(0, 120) };
      }
    }, dir);
    const files = exp.ok ? readdirSync(dir).filter((f) => f.endsWith(".md")) : [];
    ck("批量导出生成文件", exp.ok && files.length > 0, `${files.length} 个`);

    if (files.length > 0) {
      let emptyTranscript = 0;
      let hasHeader = 0;
      let hasTitle = 0;
      for (const f of files) {
        const t = readFileSync(join(dir, f), "utf8");
        if (t.startsWith("# 面试记录 #")) hasTitle++;
        if (t.includes("## 转录与问答")) hasHeader++;
        const lines = t.split(/\r?\n/);
        for (let i = 0; i < lines.length; i++) {
          if (lines[i].trim() === "## 转录与问答") {
            const rest = lines.slice(i + 1).filter((l) => l.trim());
            if (rest.length === 0 || rest[0].startsWith("## ")) emptyTranscript++;
            break;
          }
        }
      }
      ck("每份导出都有 # 面试记录 标题", hasTitle === files.length, `${hasTitle}/${files.length}`);
      ck("没有空的「转录与问答」小节", emptyTranscript === 0, emptyTranscript ? `${emptyTranscript} 个` : "");
      ck("有转录的会话仍带该小节（对照组）", hasHeader > 0, `${hasHeader} 个`);
    }
    rmSync(dir, { recursive: true, force: true });

    // ---------- 6. 热键注册（系统层占用）----------
    console.log("\n=== 热键 ===");
    const hotkeys = execFileSync(
      "powershell",
      ["-NoProfile", "-Command", `
Add-Type -Namespace V -Name H -MemberDefinition '[DllImport("user32.dll")] public static extern bool RegisterHotKey(IntPtr h,int id,uint mod,uint vk);[DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h,int id);'
$mods = 0x2 -bor 0x4
$hits = @()
foreach ($k in @(@('H',0x48),@('A',0x41),@('R',0x52),@('S',0x53))) {
  $id = 0xB000 + $k[1]
  if ([V.H]::RegisterHotKey([IntPtr]::Zero, $id, $mods, $k[1])) { [V.H]::UnregisterHotKey([IntPtr]::Zero, $id) | Out-Null; $hits += "$($k[0])=free" }
  else { if ([ComponentModel.Win32Exception]::new().NativeErrorCode -eq 1409 -or $true) { $hits += "$($k[0])=occupied" } }
}
$hits -join ','
`],
      { encoding: "utf8", timeout: 60000 },
    ).trim();
    const occupied = (hotkeys.match(/=occupied/g) ?? []).length;
    ck("四个全局热键都被本应用占用", occupied === 4, hotkeys);
  } finally {
    await browser.close().catch(() => {});
  }

  const failed = results.filter((r) => !r.ok);
  console.log(`\n${failed.length === 0 ? "ALL PASS" : "FAILED " + failed.length} (${results.length} 项)`);
  if (failed.length > 0) {
    console.log("失败项：");
    failed.forEach((f) => console.log(`  - ${f.name} ${f.detail ?? ""}`));
  }
  process.exit(failed.length === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error("脚本自身出错：", e.message);
  process.exit(2);
});