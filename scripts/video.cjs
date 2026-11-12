// Records the demo video for the hackathon post: a walk through the report.
//   node scripts/video.cjs <playwright module dir> <report.html> <out dir>
// Each stop scrolls a section into view and holds it long enough to read.
const { chromium } = require(process.argv[2]);
(async () => {
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1280, height: 720 }, recordVideo: { dir: process.argv[4], size: { width: 1280, height: 720 } } });
  const p = await ctx.newPage();
  await p.goto('file://' + process.argv[3]);
  await p.waitForTimeout(2500);
  const glide = async (y, ms) => {
    const steps = 40, from = await p.evaluate(() => window.scrollY);
    for (let i = 1; i <= steps; i++) { await p.evaluate(v => window.scrollTo(0, v), from + (y - from) * i / steps); await p.waitForTimeout(ms / steps); }
  };
  const stop = async (sel, offset, hold) => {
    const y = await p.evaluate(([s, o]) => { const el = document.querySelector(s); return el ? el.getBoundingClientRect().top + window.scrollY + o : 0; }, [sel, offset]);
    await glide(y, 1400); await p.waitForTimeout(hold);
  };
  await stop('.hero', 0, 4500);                          // what it is
  await stop('#glance', -40, 4000);                      // the numbers
  await stop('.flow', -40, 4000);                        // the five steps
  await stop('#how-do-you-trust-a-score', -20, 7000);    // why it matters
  await stop('#what-happened-on-near-testnet', -20, 5000);
  await stop('#duels', -20, 3500);
  await stop('#duels details[open]', -60, 7000);         // a task, two answers, the checker, the TEE signers
  await stop('#trying-to-get-around-it', -20, 6000);     // attacks fail
  await stop('#money', -20, 4500);
  await stop('#why-each-choice', -20, 6000);
  await stop('#weights-not-just-models', -20, 5000);
  await stop('#check-it-yourself', -20, 6000);           // arena-verify: 0 broken
  await stop('#what-near-itself-answers', -20, 5000);
  await ctx.close(); await b.close();
})();
