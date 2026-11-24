// 真实 UI 验证：配置保存链路。
// 通过 WebView2 远程调试端口连上 hint 主窗口，改一个字段 → 点「保存全部配置」
// → 读回 settings.json 确认落盘，并检查日志里出现「配置已保存」。
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const APPDATA = process.env.APPDATA;
const SETTINGS = path.join(APPDATA, "com.ojbkxc.hint", "settings.json");
const LOG = path.join(APPDATA, "com.ojbkxc.hint", "logs", "hint.log");

function readSettings() {
  return JSON.parse(fs.readFileSync(SETTINGS, "utf8"));
}

(async () => {
  const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
  // 三个窗口同源同标题：主窗口 dashboard-root / capture-root / overlay-root。
  // 必须按 DOM 特征挑，不能按顺序取（overlay 常是最后一个）。
  let page = null;
  for (const ctx of browser.contexts()) {
    for (const p of ctx.pages()) {
      const isDashboard = await p
        .evaluate(() => !!document.querySelector(".dashboard-root"))
        .catch(() => false);
      if (isDashboard) page = p;
    }
  }
  if (!page) throw new Error("没有找到主窗口（.dashboard-root）");

  console.log("== 页面标题:", await page.title());
  // 等设置页出来（骨架消失 = 配置已加载）
  await page.waitForSelector(".settings-rail", { timeout: 20000 });
  console.log("== 设置页已渲染");

  const before = readSettings();
  console.log("保存前 overlayOpacity =", before.overlayOpacity);

  // 目标：把浮层不透明度从当前值改成另一个值，走真实滑块 + 保存按钮。
  await page.click('.settings-rail-item:has-text("回答行为")');
  await page.waitForSelector('input[type="range"]', { timeout: 10000 });

  const slider = page.locator('input[type="range"]').first();
  // 滑块以「百分比整数」为值域（20–100），不是 0–1 小数。传 0.45 会被
  // HTML 输入钳到 min=20 —— 上一版就这样把目标值改成了 20%。
  const targetPercent = before.overlayOpacity > 0.6 ? 45 : 95;
  const expect = targetPercent / 100;
  await slider.evaluate((el, v) => {
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      "value",
    ).set;
    setter.call(el, String(v));
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }, targetPercent);
  await page.waitForTimeout(300);
  console.log("== 滑块已设为", targetPercent, "（期望落盘", expect, "）");

  // 日志基线
  const logBefore = fs.existsSync(LOG)
    ? fs.readFileSync(LOG, "utf8").split("\n").length
    : 0;

  // 点「保存全部配置」
  const saveBtn = page.locator('button:has-text("保存全部配置")');
  await saveBtn.waitFor({ timeout: 10000 });
  await saveBtn.click();
  console.log("== 已点击保存");

  // 等状态提示出现（成功或失败都算）
  await page
    .waitForSelector('.status, .error-text, .status-warning', { timeout: 20000 })
    .catch(() => {});
  await page.waitForTimeout(1500);

  const after = readSettings();
  console.log("保存后 overlayOpacity =", after.overlayOpacity);

  const statusText = await page
    .locator(".status, .error-text, .status-warning")
    .first()
    .textContent()
    .catch(() => null);
  console.log("界面状态提示:", JSON.stringify(statusText));

  const newLog = fs.existsSync(LOG)
    ? fs
        .readFileSync(LOG, "utf8")
        .split("\n")
        .slice(logBefore)
        .filter((l) => l.includes("配置已保存") || l.includes("保存配置失败"))
    : [];
  console.log("新增保存日志:", newLog.length ? newLog : "(无)");

  const ok = Math.abs(after.overlayOpacity - target) < 0.02;
  console.log(ok ? "RESULT=PASS 配置真的落盘了" : "RESULT=FAIL 配置没有落盘");
  await browser.close();
  process.exit(ok ? 0 : 1);
})().catch((e) => {
  console.error("ERROR:", e.message);
  process.exit(2);
});
