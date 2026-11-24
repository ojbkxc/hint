// 关键诊断：当前页面里 __TAURI_INTERNALS__ 到底存不存在、invoke 能否用，
// 以及 isTauriAsync 的等待逻辑是否真的被走到了。
const { chromium } = require("playwright");
(async () => {
  const b = await chromium.connectOverCDP("http://127.0.0.1:9222");
  console.log("targets:");
  for (const ctx of b.contexts()) {
    for (const p of ctx.pages()) {
      const info = await p
        .evaluate(() => ({
          url: location.href,
          readyState: document.readyState,
          hasInternals: "__TAURI_INTERNALS__" in window,
          internalsType: typeof window.__TAURI_INTERNALS__,
          invokeType: typeof window.__TAURI_INTERNALS__?.invoke,
          hasRoot: !!document.querySelector("#root"),
          rootFirstChild: document.querySelector("#root")?.firstElementChild
            ?.className,
        }))
        .catch((e) => ({ err: e.message }));
      console.log(" ", JSON.stringify(info));
    }
  }

  // 在 dashboard 上直接跑一次与产品相同的判断路径
  let page = null;
  for (const ctx of b.contexts()) {
    for (const p of ctx.pages()) {
      const ok = await p
        .evaluate(() => !!document.querySelector(".dashboard-root"))
        .catch(() => false);
      if (ok) page = p;
    }
  }
  if (page) {
    const probe = await page.evaluate(async () => {
      const out = {};
      out.hasInternals = "__TAURI_INTERNALS__" in window;
      // 模拟 isTauriAsync 的轮询（产品里最多等 1.5s）
      const start = Date.now();
      while (Date.now() - start < 2000 && !("__TAURI_INTERNALS__" in window)) {
        await new Promise((r) => setTimeout(r, 50));
      }
      out.waitedMs = Date.now() - start;
      out.hasAfterWait = "__TAURI_INTERNALS__" in window;
      try {
        const r = await window.__TAURI_INTERNALS__.invoke("get_settings");
        out.invokeOk = true;
        out.baseUrl = r?.baseUrl;
      } catch (e) {
        out.invokeErr = String(e?.message ?? e);
      }
      return out;
    });
    console.log("probe:", JSON.stringify(probe));
  }

  await b.close();
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(2);
});