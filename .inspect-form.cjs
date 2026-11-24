// 诊断：主窗口渲染后，表单里的 baseUrl/model 到底是什么？
// 不点保存、不改任何东西——只读 DOM，判断「加载后 form 是否被填上」。
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
  await page.waitForSelector(".settings-rail", { timeout: 20000 });
  await page.waitForTimeout(1500);

  // 默认选中的是「模型接口」section，直接读它的输入框
  const inputs = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll(".settings-panel input, .settings-panel select").forEach((el) => {
      const label =
        el.getAttribute("aria-label") ||
        el.closest(".field")?.querySelector(".field-label")?.textContent ||
        "";
      out.push({
        label: (label || "").trim().slice(0, 24),
        value: el.value,
        id: el.id || "",
      });
    });
    return out;
  });
  console.log("== 模型接口 section 的输入框 ==");
  for (const i of inputs) {
    console.log(`  [${i.id}] ${i.label} = ${JSON.stringify(i.value)}`);
  }

  // 再看 KV 区（当前生效）里显示的 baseUrl
  const kv = await page.evaluate(() => {
    const rows = [];
    document.querySelectorAll(".settings-kv").forEach((block) => {
      const keys = block.querySelectorAll(".settings-kv-key");
      const vals = block.querySelectorAll(".settings-kv-val");
      keys.forEach((k, i) => rows.push([k.textContent.trim(), vals[i]?.textContent.trim()]));
    });
    return rows;
  });
  console.log("== 当前生效 KV ==");
  for (const [k, v] of kv) console.log(`  ${k}: ${v}`);

  await b.close();
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(2);
});
