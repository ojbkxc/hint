// 抓前端 console / pageerror / CSP 违规，判断 IPC 为什么失败。
const { chromium } = require("playwright");
(async () => {
  const b = await chromium.connectOverCDP("http://127.0.0.1:9222");
  let page = null;
  for (const ctx of b.contexts()) {
    for (const p of ctx.pages()) {
      const ok = await p
        .evaluate(() => !!document.querySelector(".dashboard-root"))
        .catch(() => false);
      if (ok) page = p;
    }
  }
  if (!page) throw new Error("no dashboard page");

  const logs = [];
  page.on("console", (m) => logs.push(`[console.${m.type()}] ${m.text()}`));
  page.on("pageerror", (e) => logs.push(`[pageerror] ${e.message}`));

  // IPC 是否可用
  const ipc = await page.evaluate(() => ({
    hasInternals: typeof window.__TAURI_INTERNALS__ !== "undefined",
    hasInvoke:
      typeof window.__TAURI_INTERNALS__?.invoke === "function",
    keys: window.__TAURI_INTERNALS__
      ? Object.keys(window.__TAURI_INTERNALS__).slice(0, 10)
      : [],
  }));
  console.log("== IPC 状态 ==", JSON.stringify(ipc));

  // 主动试一次 invoke，看真实错误
  const probe = await page.evaluate(async () => {
    try {
      if (!window.__TAURI_INTERNALS__?.invoke) return { err: "no invoke" };
      const r = await window.__TAURI_INTERNALS__.invoke("get_settings");
      return { ok: true, keys: Object.keys(r || {}), baseUrl: r?.baseUrl };
    } catch (e) {
      return { err: String(e && e.message ? e.message : e) };
    }
  });
  console.log("== 直接 invoke('get_settings') ==", JSON.stringify(probe));

  // 重新加载页面，把 console 事件抓全
  await page.reload({ waitUntil: "domcontentloaded" }).catch(() => {});
  await page.waitForTimeout(3000);
  console.log("== 重载后的 console ==");
  if (!logs.length) console.log("  (none)");
  for (const l of logs.slice(0, 25)) console.log("  " + l);

  await b.close();
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(2);
});
