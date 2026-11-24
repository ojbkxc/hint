// 读设置页「读取失败」的具体错误文本，并复现首屏 effect 的真实时序。
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
  if (!page) throw new Error("no dashboard");

  // 界面上的状态文本（含「读取失败：xxx」）
  const texts = await page.evaluate(() => {
    const out = [];
    document
      .querySelectorAll(".error-text, .status, .status-warning, .muted.small")
      .forEach((el) => {
        const t = (el.textContent || "").trim();
        if (t) out.push(t.slice(0, 200));
      });
    return out;
  });
  console.log("== 界面状态文本 ==");
  for (const t of texts) console.log("  " + t);

  // reload 并在最开始就记录 __TAURI_INTERNALS__ 出现的时间
  console.log("\n== reload 并观测 IPC 注入时机 ==");
  await page.addInitScript(() => {
    window.__IPC_PROBE = { at0: "__TAURI_INTERNALS__" in window, samples: [] };
    const t0 = Date.now();
    const timer = setInterval(() => {
      const has = "__TAURI_INTERNALS__" in window;
      window.__IPC_PROBE.samples.push([Date.now() - t0, has]);
      if (has) clearInterval(timer);
    }, 10);
    setTimeout(() => clearInterval(timer), 4000);
  });
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForTimeout(4000);
  const probe = await page.evaluate(() => window.__IPC_PROBE);
  console.log("  at0:", probe.at0);
  const firstTrue = probe.samples.find(([, has]) => has);
  console.log("  IPC 首次出现: 约", firstTrue ? firstTrue[0] + "ms" : "未出现", "（采样数", probe.samples.length, "）");

  await b.close();
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(2);
});