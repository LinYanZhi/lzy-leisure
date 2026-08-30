import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useData } from "@glbt/appkit-ui";
import { api, type Comic, type PageInfo } from "../../api";
import { store } from "../../data";
import PdfReader from "./PdfReader";

// 页面 data URL 内存缓存（同一会话内避免重复读取）
const pageCache = new Map<string, string>();

interface Props {
  comic: Comic;
  onClose: () => void;
}

/**
 * 阅读器入口：
 *  - 内部管理当前章节，支持在阅读中直接切换上一章 / 下一章
 *  - 图片章节走 ImageReader（沉浸式滚动），PDF 章节走 PdfReader
 */
export default function ReaderView({ comic, onClose }: Props) {
  const [chapter, setChapter] = useState<Comic>(comic);
  const [chapters, setChapters] = useState<Comic[]>([]);

  // 父组件打开其他章节时重置
  useEffect(() => {
    setChapter(comic);
  }, [comic.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // 载入本系列章节列表（用于上一章/下一章）
  useEffect(() => {
    if (!comic.series_id) {
      setChapters([]);
      return;
    }
    api
      .getChapters(comic.series_id)
      .then(setChapters)
      .catch(() => setChapters([]));
  }, [comic.series_id]);

  const goChapter = useCallback(
    (dir: number) => {
      const idx = chapters.findIndex((c) => c.id === chapter.id);
      const next = chapters[idx + dir];
      if (next) setChapter(next);
    },
    [chapters, chapter.id],
  );

  const chapterIdx = chapters.findIndex((c) => c.id === chapter.id);

  if (chapter.type === "pdf") {
    return <PdfReader comic={chapter} onClose={onClose} />;
  }
  return (
    <ImageReader
      key={chapter.id}
      comic={chapter}
      onClose={onClose}
      onGoChapter={goChapter}
      canPrevChapter={chapterIdx > 0}
      canNextChapter={chapterIdx >= 0 && chapterIdx < chapters.length - 1}
      chapters={chapters}
      chapterIdx={chapterIdx}
    />
  );
}

// ══════════════════════════════════════════════════════════
//  图片章节：沉浸式滚动阅读器
// ══════════════════════════════════════════════════════════

interface ImageReaderProps {
  comic: Comic;
  onClose: () => void;
  onGoChapter: (dir: number) => void;
  canPrevChapter: boolean;
  canNextChapter: boolean;
  /** 系列章节列表（用于无痕滚动拼接后续章节） */
  chapters: Comic[];
  /** comic 在 chapters 中的索引 */
  chapterIdx: number;
}

/**
 * 外层：持有"无痕滚动"开关（localStorage 持久化）。
 * 开关切换不重建内层，由内层原地裁剪拼接队列 / 恢复自动拼接。
 */
function ImageReader(props: ImageReaderProps) {
  const [seamlessRaw, setSeamless] = useData(store.keys["reader-seamless"]);
  const [stepRaw, setStep] = useData(store.keys["reader-scroll-step"]);
  const seamless = seamlessRaw === "1";
  const stepPct = Math.min(100, Math.max(1, parseInt(stepRaw, 10) || 10));
  return (
    <ImageReaderInner
      {...props}
      seamless={seamless}
      onToggleSeamless={() => setSeamless(seamless ? "0" : "1")}
      stepPct={stepPct}
      onStepChange={(v) => setStep(String(v))}
    />
  );
}

interface ImageReaderInnerProps extends ImageReaderProps {
  seamless: boolean;
  onToggleSeamless: () => void;
  /** 滚轮步长（视口高度百分比，1-100） */
  stepPct: number;
  onStepChange: (v: number) => void;
}

/** 无痕滚动时，距顶部/底部该距离内提前拼接上一/下一章（保证衔接处无等待） */
const SEAMLESS_TRIGGER = 1600;

/** 无痕滚动拼接队列中的一章：页面 + 该章第一页在拼接 DOM 中的全局索引 */
interface QueuedChapter {
  comic: Comic;
  pages: PageInfo[];
  pageOffset: number;
}

/** 元素相对滚动容器内容原点的纵坐标（与 scrollTop 同基准） */
function contentTop(el: HTMLElement, c: HTMLElement): number {
  return el.getBoundingClientRect().top - c.getBoundingClientRect().top + c.scrollTop;
}

/** 章内阅读进度（0~1）：按章内可滚动范围计算；末页未挂载时按页占比兜底 */
function chapterProgressPct(
  c: HTMLElement,
  refs: (HTMLDivElement | null)[],
  entry: QueuedChapter,
  pageInChapter: number,
): number {
  const first = refs[entry.pageOffset];
  const last = refs[entry.pageOffset + entry.pages.length - 1];
  if (first && last && last.offsetHeight > 0) {
    const chapTop = contentTop(first, c);
    const chapBottom = contentTop(last, c) + last.offsetHeight;
    const scrollable = Math.max(1, chapBottom - chapTop - c.clientHeight);
    return Math.max(0, Math.min(1, (c.scrollTop - chapTop) / scrollable));
  }
  return (pageInChapter + 0.5) / entry.pages.length;
}

function ImageReaderInner({
  comic,
  onClose,
  onGoChapter,
  canPrevChapter,
  canNextChapter,
  chapters,
  chapterIdx,
  seamless,
  onToggleSeamless,
  stepPct,
  onStepChange,
}: ImageReaderInnerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const pageRefs = useRef<(HTMLDivElement | null)[]>([]);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const restoredRef = useRef(false);
  const pctAdjustedRef = useRef(false);
  const initialPctRef = useRef<number | null>(null);
  const currentPageRef = useRef(0);
  // 恢复后用户是否已手动滚动（滚动后不再强制修正滚动位置，避免阅读中跳动）
  const userScrolledRef = useRef(false);
  // ── 无痕滚动：章节拼接队列 ──
  const [queue, setQueue] = useState<QueuedChapter[]>([]);
  const queueRef = useRef<QueuedChapter[]>([]);
  const appendingRef = useRef(false);
  const prependingRef = useRef(false);
  const noMoreRef = useRef(false);
  const noMorePrevRef = useRef(false);
  const [noMore, setNoMore] = useState(false);
  // 队首章在 chapters 中的索引（向上拼接会减、关闭开关会复位）。
  // 注意：章节列表（chapters）可能晚于组件首次渲染加载完成，此时 chapterIdx=-1，
  // 若在此刻用 useRef(chapterIdx) 初始化会被永久钉死为 -1，导致下一章定位到 chapters[0]
  // （109 话后接到第 1 话）。故初始为 null，读取时惰性取当前 chapterIdx，直到用户操作落定。
  const loIdxRef = useRef<number | null>(null);
  // prepend 后待执行的滚动位置修正（旧 scrollTop / 旧 scrollHeight）
  const prependPendingRef = useRef<{ oldScrollTop: number; oldScrollHeight: number } | null>(null);
  // 当前所在章节（queue 内索引），用于工具栏显示章节名/章内页码
  const chapterIdxRef = useRef(0);
  const [curChapterIdx, setCurChapterIdx] = useState(0);

  const [currentPage, setCurrentPage] = useState(0);
  const [progressPct, setProgressPct] = useState(0);
  const [loading, setLoading] = useState(true);
  // 沉浸式控件：顶部工具栏 + 底部控制条，默认自动隐藏，点击画面呼出
  const [uiVisible, setUiVisible] = useState(true);
  const uiTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const downPos = useRef<{ x: number; y: number } | null>(null);

  // 拼接队列中的当前章节
  const curChapter: QueuedChapter | undefined = queue[curChapterIdx];
  const totalPages = queue.reduce((s, ch) => s + ch.pages.length, 0);

  // 载入初始章节页列表与历史进度
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [pageList, config] = await Promise.all([
        api.listPages(comic.id),
        api.getComicConfig(comic.id),
      ]);
      if (cancelled) return;
      const first: QueuedChapter = { comic, pages: pageList, pageOffset: 0 };
      setQueue([first]);
      queueRef.current = [first];
      setLoading(false);
      // 恢复位置：优先滚动百分比，其次页码
      const pct = Number(config["scroll_pct"] ?? 0);
      if (pct > 0) {
        initialPctRef.current = Math.min(1, pct / 1000);
      } else if (comic.reading_progress > 0) {
        currentPageRef.current = comic.reading_progress;
      }
      requestAnimationFrame(() => {
        restoredRef.current = true;
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [comic.id]);

  // 无痕滚动：追加下一章到拼接队列（仅在无缝模式下调用）
  const appendNext = useCallback(async () => {
    if (appendingRef.current || noMoreRef.current) return;
    if (chapters.length === 0) return; // 章节列表未就绪：不置 noMore，等加载完成后重试
    const q = queueRef.current;
    const lo = loIdxRef.current ?? chapterIdx;
    const nextIdx = lo + q.length;
    if (nextIdx < 0 || nextIdx >= chapters.length) {
      noMoreRef.current = true;
      setNoMore(true);
      return;
    }
    const next = chapters[nextIdx];
    // 仅图片章节（folder/archive）可拼接；PDF / 子系列为拼接边界
    if (next.type !== "folder" && next.type !== "archive") {
      noMoreRef.current = true;
      setNoMore(true);
      return;
    }
    appendingRef.current = true;
    try {
      const pages = await api.listPages(next.id);
      if (pages.length === 0) {
        noMoreRef.current = true;
        setNoMore(true);
        return;
      }
      const last = q[q.length - 1];
      const entry: QueuedChapter = {
        comic: next,
        pages,
        pageOffset: last.pageOffset + last.pages.length,
      };
      const q2 = [...q, entry];
      queueRef.current = q2;
      setQueue(q2);
    } catch {
      noMoreRef.current = true;
      setNoMore(true);
    } finally {
      appendingRef.current = false;
    }
  }, [chapters, chapterIdx]);

  // 无痕滚动：把上一章插入到拼接队列头部（仅在无缝模式下调用）
  const prependPrev = useCallback(async () => {
    if (prependingRef.current || noMorePrevRef.current) return;
    if (chapters.length === 0) return; // 章节列表未就绪：不置 noMorePrev，等加载完成后重试
    const lo = loIdxRef.current ?? chapterIdx;
    const prevIdx = lo - 1;
    if (prevIdx < 0) {
      noMorePrevRef.current = true;
      return;
    }
    const prev = chapters[prevIdx];
    // 仅图片章节（folder/archive）可拼接；PDF / 子系列为拼接边界
    if (prev.type !== "folder" && prev.type !== "archive") {
      noMorePrevRef.current = true;
      return;
    }
    prependingRef.current = true;
    const c = containerRef.current;
    const oldScrollTop = c ? c.scrollTop : 0;
    const oldScrollHeight = c ? c.scrollHeight : 0;
    try {
      const pages = await api.listPages(prev.id);
      if (pages.length === 0) {
        noMorePrevRef.current = true;
        return;
      }
      const shift = pages.length;
      const q = queueRef.current;
      const q2: QueuedChapter[] = [
        { comic: prev, pages, pageOffset: 0 },
        ...q.map((e) => ({ ...e, pageOffset: e.pageOffset + shift })),
      ];
      queueRef.current = q2;
      setQueue(q2);
      loIdxRef.current = prevIdx;
      // 全局页索引 / 所在章节索引整体后移，保持当前页不变
      currentPageRef.current += shift;
      setCurrentPage((p) => p + shift);
      chapterIdxRef.current += 1;
      setCurChapterIdx(chapterIdxRef.current);
      // DOM 提交后把滚动位置下移新章高度，保持视图内容不动
      prependPendingRef.current = { oldScrollTop, oldScrollHeight };
    } catch {
      noMorePrevRef.current = true;
    } finally {
      prependingRef.current = false;
    }
  }, [chapters, chapterIdx]);

  // prepend 后修正滚动位置：滚动偏移增加新章高度差（useLayoutEffect 保证在绘制前完成）
  useLayoutEffect(() => {
    if (!prependPendingRef.current) return;
    const c = containerRef.current;
    if (!c) return;
    const { oldScrollTop, oldScrollHeight } = prependPendingRef.current;
    c.scrollTop = oldScrollTop + (c.scrollHeight - oldScrollHeight);
    prependPendingRef.current = null;
  }, [queue]);

  // 滚动接近底部时提前拼接下一章，保证衔接处无等待
  const maybeAppend = useCallback(() => {
    if (!seamless || noMoreRef.current) return;
    const c = containerRef.current;
    if (!c) return;
    if (c.scrollTop + c.clientHeight >= c.scrollHeight - SEAMLESS_TRIGGER) {
      appendNext();
    }
  }, [seamless, appendNext]);

  // 滚动接近顶部时提前拼接上一章
  const maybePrepend = useCallback(() => {
    if (!seamless || noMorePrevRef.current) return;
    const c = containerRef.current;
    if (!c) return;
    if (c.scrollTop <= SEAMLESS_TRIGGER) {
      prependPrev();
    }
  }, [seamless, prependPrev]);

  // 章节列表加载完成 / 开关切换后补一次拼接检查：
  // 打开时若已近底部/顶部（如恢复位置恰在章末），此时列表若尚未就绪会漏接，这里兜底
  useEffect(() => {
    if (!seamless || chapters.length === 0) return;
    requestAnimationFrame(() => {
      maybeAppend();
      maybePrepend();
    });
  }, [chapters, seamless, maybeAppend, maybePrepend]);

  // 关闭无痕开关：把拼接队列裁剪到当前所在章节，原地回到单章模式
  useEffect(() => {
    if (seamless) return;
    const q = queueRef.current;
    if (q.length <= 1) return;
    const cur = chapterIdxRef.current;
    const entry = q[cur];
    if (!entry || entry.pages.length === 0) return;
    const c = containerRef.current;
    const p = Math.max(0, Math.min(currentPageRef.current - entry.pageOffset, entry.pages.length - 1));
    const single: QueuedChapter[] = [{ comic: entry.comic, pages: entry.pages, pageOffset: 0 }];
    queueRef.current = single;
    setQueue(single);
    // 队首章在 chapters 中的索引复位为当前章，之后自动拼接从当前章继续
    loIdxRef.current = (loIdxRef.current ?? chapterIdx) + cur;
    chapterIdxRef.current = 0;
    setCurChapterIdx(0);
    noMoreRef.current = false;
    setNoMore(false);
    noMorePrevRef.current = false;
    // 队列收缩后重新定位到当前页
    requestAnimationFrame(() => {
      const el = pageRefs.current[p];
      if (el && c) c.scrollTop = el.offsetTop - 12;
    });
  }, [seamless]);

  // 页面全部挂载后恢复滚动位置
  useEffect(() => {
    if (totalPages === 0 || !restoredRef.current) return;
    const c = containerRef.current;
    if (!c) return;
    if (initialPctRef.current != null) {
      c.scrollTop = (c.scrollHeight - c.clientHeight) * initialPctRef.current;
    } else if (currentPageRef.current > 0) {
      const el = pageRefs.current[currentPageRef.current];
      if (el) c.scrollTop = el.offsetTop - 20;
    }
    restoredRef.current = false; // 仅恢复一次
    // 恢复后检查一次是否需要拼接（如章节很短、恢复位置已近底部）
    requestAnimationFrame(maybeAppend);
  }, [totalPages, maybeAppend]);

  // 图片加载完成后修正百分比定位（此时高度才准确）；用户已滚动则跳过，避免跳动
  const onImageLoad = useCallback(() => {
    if (userScrolledRef.current) return;
    const c = containerRef.current;
    if (!c) return;
    if (initialPctRef.current != null && !pctAdjustedRef.current) {
      c.scrollTop = (c.scrollHeight - c.clientHeight) * initialPctRef.current;
      pctAdjustedRef.current = true;
    }
  }, []);

  const scheduleSave = useCallback((comicId: string, page: number, pct: number) => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      api.setReadingProgress(comicId, page).catch(() => {});
      api.setComicConfig(comicId, { scroll_pct: String(Math.round(pct * 1000)) }).catch(() => {});
    }, 500);
  }, []);

  const handleScroll = useCallback(() => {
    const c = containerRef.current;
    if (!c) return;
    userScrolledRef.current = true; // 用户已滚动，之后不再强制修正恢复位置
    const mid = c.scrollTop + c.clientHeight / 2;
    let cur = 0;
    const refs = pageRefs.current;
    for (let i = 0; i < refs.length; i++) {
      const el = refs[i];
      if (!el) continue; // 未挂载的页跳过，避免提前中断导致页码回跳
      if (el.offsetTop <= mid) cur = i;
      else break;
    }
    currentPageRef.current = cur;
    setCurrentPage(cur);
    // ── 无痕滚动：定位所在章节，跨章时保存进度 ──
    const q = queueRef.current;
    let ci = 0;
    for (let i = 0; i < q.length - 1; i++) {
      if (cur >= q[i + 1].pageOffset) ci = i + 1;
      else break;
    }
    if (ci !== chapterIdxRef.current) {
      const old = q[chapterIdxRef.current];
      // 向下滚入新章节时，把上一章标记为读完（向上回滚不标记）
      if (ci > chapterIdxRef.current && old && old.pages.length > 0) {
        api.setReadingProgress(old.comic.id, old.pages.length).catch(() => {});
      }
      chapterIdxRef.current = ci;
      setCurChapterIdx(ci);
    }
    const entry = q[ci];
    if (!entry || entry.pages.length === 0) return;
    const pageInChapter = Math.max(0, Math.min(cur - entry.pageOffset, entry.pages.length - 1));
    // 章内进度：单章保持原高度比（零回归）；多章拼接用章内高度比，末页未挂载时按页占比兜底
    const chapPct =
      q.length === 1
        ? c.scrollHeight > c.clientHeight
          ? c.scrollTop / (c.scrollHeight - c.clientHeight)
          : 1
        : chapterProgressPct(c, refs, entry, pageInChapter);
    setProgressPct(chapPct);
    // 进度为 0-based 页索引 + 章内百分比（拼接不影响该章自身进度）
    scheduleSave(entry.comic.id, pageInChapter, chapPct);
    maybeAppend();
    maybePrepend();
  }, [scheduleSave, maybeAppend, maybePrepend]);

  // 滚轮步长：拦截原生滚动，一格滚动距离 = 视口高度 × stepPct%（与选封面步长同语义）
  useEffect(() => {
    const c = containerRef.current;
    if (!c) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= 16; // 行 → 像素
      else if (e.deltaMode === 2) dy *= c.clientHeight; // 页 → 像素
      const stepPx = Math.max(1, (c.clientHeight * stepPct) / 100);
      c.scrollTop += (dy * stepPx) / 100;
    };
    c.addEventListener("wheel", onWheel, { passive: false });
    return () => c.removeEventListener("wheel", onWheel);
  }, [stepPct]);

  // ── 沉浸式控件：呼出 / 自动隐藏 ──
  const showUi = useCallback(() => {
    setUiVisible(true);
    if (uiTimer.current) clearTimeout(uiTimer.current);
    uiTimer.current = setTimeout(() => setUiVisible(false), 3500);
  }, []);

  const toggleUi = useCallback(() => {
    setUiVisible((v) => {
      const next = !v;
      if (uiTimer.current) clearTimeout(uiTimer.current);
      if (next) {
        uiTimer.current = setTimeout(() => setUiVisible(false), 3500);
      } else {
        uiTimer.current = null;
      }
      return next;
    });
  }, []);

  useEffect(() => {
    showUi(); // 进入阅读先显示一次控件，随后自动隐藏
    return () => {
      if (uiTimer.current) clearTimeout(uiTimer.current);
    };
  }, [showUi]);

  // ── 上一张 / 下一张 / 拖动跳转（滚动式：按页滚动） ──
  const goTo = useCallback(
    (dir: number) => {
      const c = containerRef.current;
      if (!c) return;
      const target = Math.min(totalPages - 1, Math.max(0, currentPageRef.current + dir));
      const el = pageRefs.current[target];
      if (el) c.scrollTo({ top: el.offsetTop - 12, behavior: "smooth" });
    },
    [totalPages],
  );

  const seekTo = useCallback((p: number) => {
    const c = containerRef.current;
    const el = pageRefs.current[p];
    if (c && el) c.scrollTop = el.offsetTop - 12;
  }, []);

  // 点击检测：位移超过阈值视为滑动（不切换控件）
  const onPointerDown = (e: React.PointerEvent) => {
    downPos.current = { x: e.clientX, y: e.clientY };
  };
  const onTap = (e: React.MouseEvent) => {
    const d = downPos.current;
    if (d && (Math.abs(e.clientX - d.x) > 8 || Math.abs(e.clientY - d.y) > 8)) return;
    toggleUi();
  };

  // ESC 返回 / ← → 翻页
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowRight") goTo(1);
      else if (e.key === "ArrowLeft") goTo(-1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, goTo]);

  // 卸载时保存一次（保存当前所在章节的进度）
  useEffect(() => {
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      const q = queueRef.current;
      const entry = q[chapterIdxRef.current];
      if (entry && entry.pages.length > 0) {
        const p = Math.max(0, Math.min(currentPageRef.current - entry.pageOffset, entry.pages.length - 1));
        api.setReadingProgress(entry.comic.id, p).catch(() => {});
      }
    };
  }, [comic.id]);

  return (
    <div className="reader-overlay">
      <div className={`reader-toolbar${uiVisible ? "" : " hidden"}`}>
        <button className="btn-ghost" onClick={onClose}>← 返回</button>
        <div className="reader-title" title={curChapter?.comic.title ?? comic.title}>
          {curChapter?.comic.title ?? comic.title}
        </div>
        <label className="reader-seamless" title="无痕滚动：滚到章节末尾自动衔接下一章">
          <input type="checkbox" checked={seamless} onChange={onToggleSeamless} />
          无痕
        </label>
        <label className="reader-step" title="滚轮步长：鼠标滚一格滚动的距离（视口高度百分比）">
          <input
            type="range"
            min={1}
            max={100}
            step={1}
            value={stepPct}
            onChange={(e) => {
              const v = Math.min(100, Math.max(1, parseInt(e.target.value, 10) || 10));
              onStepChange(v);
            }}
          />
          <span>{stepPct}%</span>
        </label>
        <div className="reader-meta">
          {loading
            ? "加载中…"
            : curChapter && curChapter.pages.length > 0
              ? `${currentPage - curChapter.pageOffset + 1} / ${curChapter.pages.length} · ${Math.round(progressPct * 100)}%`
              : ""}
        </div>
      </div>

      <div
        className="reader-scroll"
        ref={containerRef}
        onScroll={handleScroll}
        onPointerDown={onPointerDown}
        onClick={onTap}
      >
        {loading ? (
          <div className="center-hint">正在加载页面列表…</div>
        ) : totalPages === 0 ? (
          <div className="center-hint">该章节暂不支持阅读（rar/cbr 后续支持）</div>
        ) : (
          queue.flatMap((ch) =>
            ch.pages.map((p, i) => (
              <ReaderPage
                key={`${ch.comic.id}-${p.index}`}
                comicId={ch.comic.id}
                index={p.index}
                onLoad={onImageLoad}
                innerRef={(el) => (pageRefs.current[ch.pageOffset + i] = el)}
              />
            )),
          )
        )}
        {(seamless ? noMore : totalPages > 0) && (
          <div className="reader-end" onClick={(e) => { e.stopPropagation(); onClose(); }}>
            已到末尾，点击返回书架
          </div>
        )}
      </div>

      {totalPages > 0 && !loading && (
        <div className={`reader-controls${uiVisible ? "" : " hidden"}`}>
          <button className="reader-nav-btn" onClick={() => onGoChapter(-1)} disabled={!canPrevChapter}>上一章</button>
          <button className="reader-nav-btn only-desktop" onClick={() => goTo(-1)} disabled={currentPage === 0}>上一张</button>
          <input
            className="reader-seek"
            type="range"
            min={0}
            max={totalPages - 1}
            value={currentPage}
            onChange={(e) => seekTo(Number(e.target.value))}
          />
          <span className="reader-pageno">{currentPage + 1} / {totalPages}</span>
          <button className="reader-nav-btn only-desktop" onClick={() => goTo(1)} disabled={currentPage >= totalPages - 1}>下一张</button>
          <button className="reader-nav-btn" onClick={() => onGoChapter(1)} disabled={!canNextChapter}>下一章</button>
        </div>
      )}
    </div>
  );
}

interface PageProps {
  comicId: string;
  index: number;
  onLoad: () => void;
  innerRef: (el: HTMLDivElement | null) => void;
}

/** 单页图片（进入视口时按需读取） */
function ReaderPage({ comicId, index, onLoad, innerRef }: PageProps) {
  const elRef = useRef<HTMLDivElement | null>(null);
  const [url, setUrl] = useState<string>();

  useEffect(() => {
    const cacheKey = `${comicId}:${index}`;
    const cached = pageCache.get(cacheKey);
    if (cached) {
      setUrl(cached);
      return;
    }
    const el = elRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            io.disconnect();
            api
              .getPageDataUrl(comicId, index)
              .then((u) => {
                pageCache.set(cacheKey, u);
                setUrl(u);
              })
              .catch(() => {});
            break;
          }
        }
      },
      { rootMargin: "800px 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [comicId, index]);

  return (
    <div className="reader-page" ref={(el) => { elRef.current = el; innerRef(el); }}>
      {url ? (
        <img src={url} alt="" draggable={false} onLoad={onLoad} />
      ) : (
        <div className="reader-page-placeholder" />
      )}
    </div>
  );
}
