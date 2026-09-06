// 临时验证脚本：模拟无痕拼接在"章节列表先于初始页面列表加载完成"竞态下的行为。
// 旧实现（q[-1] 崩溃 → noMore 永久钉死）vs 新实现（空队列直接返回，稍后重试）。
// 运行：node scripts/sim-seamless-race.mjs
import assert from "node:assert";

// ── 旧实现（修复前 appendNext 的核心逻辑）──
async function appendNextOld(ctx) {
  const { chapters, chapterIdx, queueRef, appendingRef, noMoreRef, setNoMore } = ctx;
  if (appendingRef.current || noMoreRef.current) return;
  if (chapters.length === 0) return;
  const q = queueRef.current;
  const lo = ctx.loIdxRef.current ?? chapterIdx;
  const nextIdx = lo + q.length;
  if (nextIdx < 0 || nextIdx >= chapters.length) {
    noMoreRef.current = true;
    setNoMore(true);
    return;
  }
  const next = chapters[nextIdx];
  if (next.type !== "folder" && next.type !== "archive") {
    noMoreRef.current = true;
    setNoMore(true);
    return;
  }
  appendingRef.current = true;
  try {
    const pages = await ctx.listPages(next.id);
    if (pages.length === 0) {
      noMoreRef.current = true;
      setNoMore(true);
      return;
    }
    const last = q[q.length - 1];
    const entry = { comic: next, pages, pageOffset: last.pageOffset + last.pages.length };
    const q2 = [...q, entry];
    queueRef.current = q2;
    ctx.setQueue(q2);
  } catch {
    noMoreRef.current = true;
    setNoMore(true);
  } finally {
    appendingRef.current = false;
  }
}

// ── 新实现（修复后 appendNext 的核心逻辑）──
async function appendNextNew(ctx) {
  const { chapters, chapterIdx, queueRef, appendingRef, noMoreRef, setNoMore } = ctx;
  if (appendingRef.current || noMoreRef.current) return;
  if (chapters.length === 0) return;
  const q = queueRef.current;
  if (q.length === 0) return; // 初始页面尚未加载完成：稍后重试
  const lo = ctx.loIdxRef.current ?? chapterIdx;
  if (lo < 0) return;
  const nextIdx = lo + q.length;
  if (nextIdx >= chapters.length) {
    noMoreRef.current = true;
    setNoMore(true);
    return;
  }
  appendingRef.current = true;
  try {
    let probe = nextIdx;
    let appended = false;
    while (probe < chapters.length) {
      const cand = chapters[probe];
      if (cand.type !== "folder" && cand.type !== "archive") break;
      const pages = await ctx.listPages(cand.id);
      if (pages.length > 0) {
        const last = queueRef.current[queueRef.current.length - 1];
        const entry = { comic: cand, pages, pageOffset: last.pageOffset + last.pages.length };
        const q2 = [...queueRef.current, entry];
        queueRef.current = q2;
        ctx.setQueue(q2);
        appended = true;
        break;
      }
      probe += 1;
    }
    if (!appended) {
      noMoreRef.current = true;
      setNoMore(true);
    }
  } catch {
    // 单次失败不置 noMore
  } finally {
    appendingRef.current = false;
  }
}

function makeCtx({ listPagesDelay = 50 } = {}) {
  // 章节列表：5 个 folder 章；当前章 index=2
  const chapters = [0, 1, 2, 3, 4].map((i) => ({ id: `c${i}`, type: "folder", pages: [i] }));
  return {
    chapters,
    chapterIdx: 2,
    queueRef: { current: [] }, // ← 竞态：初始页面列表还没返回，队列为空
    loIdxRef: { current: null },
    appendingRef: { current: false },
    noMoreRef: { current: false },
    noMore: false,
    setNoMore: (v) => (ctx_.noMore = v),
    setQueue: (q) => (ctx_.queue = q),
    queue: [],
    listPages: async (id) => {
      await new Promise((r) => setTimeout(r, listPagesDelay));
      const idx = Number(id.slice(1));
      return chapters[idx].pages; // 全部非空
    },
  };
}

// ── 场景 A：竞态触发（队列为空时调用 appendNext）──
{
  const ctx = makeCtx();
  ctx.setNoMore = (v) => (ctx.noMore = v);
  ctx.setQueue = (q) => (ctx.queue = q);
  await appendNextOld(ctx);
  console.log("[旧实现] 竞态后: noMore =", ctx.noMore, "| 队列长度 =", ctx.queueRef.current.length);
  assert.ok(ctx.noMore === true, "旧实现应把 noMore 钉死为 true（无缝永久失效）");
}

{
  const ctx = makeCtx();
  ctx.setNoMore = (v) => (ctx.noMore = v);
  ctx.setQueue = (q) => (ctx.queue = q);
  await appendNextNew(ctx);
  console.log("[新实现] 竞态后: noMore =", ctx.noMore, "| 队列长度 =", ctx.queueRef.current.length);
  assert.ok(ctx.noMore === false, "新实现不应设置 noMore");
  assert.ok(ctx.queueRef.current.length === 0, "新实现不应修改队列");
}

// ── 场景 B：竞态后初始页面加载完成，再滚动到末尾应正常拼接 ──
{
  const ctx = makeCtx();
  ctx.setNoMore = (v) => (ctx.noMore = v);
  ctx.setQueue = (q) => (ctx.queue = q);
  await appendNextNew(ctx); // 竞态：空队列 → 返回
  // 初始页面列表返回，队列就绪
  ctx.queueRef.current = [{ comic: ctx.chapters[2], pages: ctx.chapters[2].pages, pageOffset: 0 }];
  ctx.queue = ctx.queueRef.current;
  await appendNextNew(ctx); // 正常拼接下一章
  assert.ok(ctx.noMore === false, "正常拼接不应置 noMore");
  assert.strictEqual(ctx.queueRef.current.length, 2, "应拼入下一章");
  assert.strictEqual(ctx.queueRef.current[1].comic.id, "c3", "下一章应为 c3");
  console.log("[新实现] 竞态后正常续接: 队列 =", ctx.queueRef.current.map((e) => e.comic.id).join(","), "| noMore =", ctx.noMore);
}

// ── 场景 C：下一章为空章时跳过，接到再下一章 ──
{
  const ctx = makeCtx();
  ctx.setNoMore = (v) => (ctx.noMore = v);
  ctx.setQueue = (q) => (ctx.queue = q);
  ctx.chapters[3] = { id: "c3", type: "folder", pages: [] }; // c3 空章
  ctx.chapters[4] = { id: "c4", type: "folder", pages: [4] };
  ctx.queueRef.current = [{ comic: ctx.chapters[2], pages: [2], pageOffset: 0 }];
  await appendNextNew(ctx);
  assert.strictEqual(ctx.queueRef.current.length, 2, "应跳过空章 c3 接到 c4");
  assert.strictEqual(ctx.queueRef.current[1].comic.id, "c4");
  console.log("[新实现] 空章跳过: 队列 =", ctx.queueRef.current.map((e) => e.comic.id).join(","));
}

// ── 场景 D：下一章是 PDF（边界）→ 置 noMore ──
{
  const ctx = makeCtx();
  ctx.setNoMore = (v) => (ctx.noMore = v);
  ctx.setQueue = (q) => (ctx.queue = q);
  ctx.chapters[3] = { id: "c3", type: "pdf", pages: [] };
  ctx.queueRef.current = [{ comic: ctx.chapters[2], pages: [2], pageOffset: 0 }];
  await appendNextNew(ctx);
  assert.ok(ctx.noMore === true, "PDF 边界应置 noMore");
  assert.strictEqual(ctx.queueRef.current.length, 1);
  console.log("[新实现] PDF 边界: noMore =", ctx.noMore);
}

console.log("\n✅ 全部断言通过：竞态不再破坏无缝拼接，正常续接 / 空章跳过 / 边界行为正确。");
