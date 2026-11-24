const { chromium } = require("playwright");
(async () => {
  const b = await chromium.connectOverCDP("http://127.0.0.1:9222");
  console.log("contexts:", b.contexts().length);
  for (const [ci, ctx] of b.contexts().entries()) {
    for (const [pi, p] of ctx.pages().entries()) {
      const url = p.url();
      const title = await p.title().catch(() => "?");
      const rootHtml = await p
        .evaluate(() => {
          const r = document.querySelector("#root");
          return r ? r.innerHTML.slice(0, 200) : "(no #root)";
        })
        .catch((e) => "eval err: " + e.message);
      console.log(`--- ctx${ci} page${pi} url=${url} title=${title}`);
      console.log("    root:", rootHtml.replace(/\s+/g, " ").slice(0, 160));
    }
  }
  await b.close();
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(2);
});
