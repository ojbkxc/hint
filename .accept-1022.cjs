// 真机验收 v1.0.22：①首屏表单是否真的有值（IPC 竞态修复）②字号滑块
// ③热键徽章按钮切换浮层。
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const SETTINGS = path.join(
  process.env.APPDATA,
  "com.ojbkxc.hint",
  "settings.json",
);
const readSettings = () => JSON.parse(fs.readFileSync(SETTINGS, "utf8"));

async function dashboardPage(browser) {
  for (const ctx of browser.contexts()) {
    for (const p of ctx.pages()) {
      const ok = await p
        .evaluate(() => !!document.querySelector(".dashboard-root"))
        .catch(() => false);
      if (ok) return p;
    }
  }
  return null;
}

const results = [];
function check(name, pass, detail) {
  results.push({ name, pass, detail });
  console.log(`${pass ? "PASS" : "FAIL"}  ${name}  ${detail ?? ""}`);
}

(async () => {
  const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
  const page = await dashboardPage(browser);
  if (!page) throw new Error("no dashboard");

  // 关键 1：首屏 form 必须有值（这正是 1.0.21 之前的竞态缺陷）
  await page.waitForSelector(".settings-rail", { timeout: 20000 });
  await page.waitForTimeout(1200);
  const formVals = await page.evaluate(() => ({
    baseUrl: document.querySelector("#model-base-url")?.value ?? null,
    model: document.querySelector("#model-name")?.value ?? null,
  }));
  check(
    "首屏 Base URL 已回填（IPC 竞态修复）",
    !!formVals.baseUrl,
    JSON.stringify(formVals),
  );

  // 关键 2：字号滑块存在且默认 100%
  await page.click('.settings-rail-item:has-text("回答行为")');
  await page.waitForSelector("#behavior-font-scale", { timeout: 10000 });
  const scaleInfo = await page.evaluate(() => {
    const el = document.querySelector("#behavior-font-scale");
    const val = document.querySelector(
      "#behavior-font-scale ~ .slider-value",
    )?.textContent;
    return { min: el?.min, max: el?.max, step: el?.step, value: el?.value, shown: val };
  });
  check(
    "字号滑块存在、范围 80–160、默认 100%",
    scaleInfo.min === "80" && scaleInfo.max === "160" && scaleInfo.shown === "100%",
    JSON.stringify(scaleInfo),
  );

  // 关键 3：改字号 + 保存 → 落盘 + 浮层 CSS 变量跟着变
  const before = readSettings();
  await page.evaluate(() => {
    const el = document.querySelector("#behavior-font-scale");
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      "value",
    ).set;
    setter.call(el, "140");
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await page.waitForTimeout(300);
  await page.locator('button:has-text("保存全部配置")').click();
  await page.waitForTimeout(2500);
  const after = readSettings();
  check(
    "字号保存后落盘 overlayFontScale=1.4",
    Math.abs((after.overlayFontScale ?? -1) - 1.4) < 0.01,
    `before=${before.overlayFontScale} after=${after.overlayFontScale}`,
  );
  // Base URL / 模型名必须仍在（回归：不能再被空值覆盖）
  check(
    "保存后 Base URL / 模型名未被清空",
    !!after.baseUrl && !!after.model,
    `baseUrl=${after.baseUrl} model=${after.model}`,
  );

  // 浮层窗口的 CSS 变量应已变成 1.4
  let overlayScale = null;
  for (const ctx of browser.contexts()) {
    for (const p of ctx.pages()) {
      const isOverlay = await p
        .evaluate(() => !!document.querySelector(".overlay-root"))
        .catch(() => false);
      if (isOverlay) {
        overlayScale = await p.evaluate(() =>
          getComputedStyle(document.documentElement).getPropertyValue(
            "--overlay-font-scale",
          ),
        );
      }
    }
  }
  check(
    "浮层 CSS 变量 --overlay-font-scale 已更新",
    overlayScale !== null && Math.abs(Number(overlayScale) - 1.4) < 0.01,
    `overlay --overlay-font-scale=${JSON.stringify(overlayScale)}`,
  );

  // 关键 4：热键徽章按钮切换浮层
  await page.click('.settings-rail-item:has-text("面试会话")');
  const badge = page.locator(".hint-badge-button");
  const labelBefore = (await badge.textContent()) ?? "";
  await badge.click();
  await page.waitForTimeout(1200);
  const labelAfter = (await badge.textContent()) ?? "";
  check(
    "徽章按钮文案随浮层状态切换",
    labelBefore !== labelAfter,
    `${JSON.stringify(labelBefore.trim())} -> ${JSON.stringify(labelAfter.trim())}`,
  );

  await browser.close();
  const failed = results.filter((r) => !r.pass);
  console.log(
    `\n${failed.length === 0 ? "ALL PASS" : "FAILED: " + failed.length} (${results.length} checks)`,
  );
  process.exit(failed.length === 0 ? 0 : 1);
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(2);
});
