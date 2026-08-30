import { useCallback, useEffect, useRef, useState } from "react";
import { useData } from "@glbt/appkit-ui";
import { api, type Comic, type PageIndex } from "../../api";
import { store } from "../../data";
import { usePersistScroll } from "../../usePersistScroll";
import CoverImage from "./CoverImage";
import PdfPickCoverModal from "./PdfPickCoverModal";

interface Props {
  series: Comic;
  onBack: () => void;
  onOpenChapter: (comic: Comic) => void;
}

function formatSize(bytes: number): string {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} GB`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

// ── 预览图模块级缓存：同一章节同一页只生成一次，拖动回看立即显示 ──
const previewCache = new Map<string, string>();
const previewPending = new Map<string, Promise<string>>();

function getPreview(comicId: string, pageIdx: number): Promise<string> {
  const key = `${comicId}:${pageIdx}`;
  const hit = previewCache.get(key);
  if (hit) return Promise.resolve(hit);
  let p = previewPending.get(key);
  if (!p) {
    p = api
      .getPagePreviewDataUrl(comicId, pageIdx)
      .then((url) => {
        previewCache.set(key, url);
        return url;
      })
      .finally(() => previewPending.delete(key));
    previewPending.set(key, p);
  }
  return p;
}

/** 计算裁剪窗口附近的可见页范围（前后各 2 页缓冲） */
function computeVisible(index: PageIndex, offset: number) {
  if (index.pages.length === 0) return { first: 0, last: 0, cumBefore: 0 };
  let cum = 0;
  let pageIdx = 0;
  for (let i = 0; i < index.pages.length; i++) {
    if (offset < cum + index.pages[i].ph) {
      pageIdx = i;
      break;
    }
    cum += index.pages[i].ph;
    if (i === index.pages.length - 1) pageIdx = i;
  }
  const BUFFER = 2;
  const first = Math.max(0, pageIdx - BUFFER);
  const last = Math.min(index.pages.length - 1, pageIdx + BUFFER);
  let cumBefore = 0;
  for (let i = 0; i < first; i++) cumBefore += index.pages[i].ph;
  return { first, last, cumBefore };
}

/** 选封面弹窗：章节多页垂直拼接成条带，拖拽/滚轮移动 5:7 裁剪窗口选封面 */
function PickCoverModal({
  comic,
  onPicked,
  onClose,
}: {
  comic: Comic;
  onPicked: () => void;
  onClose: () => void;
}) {
  const [index, setIndex] = useState<PageIndex | null>(null);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [containerH, setContainerH] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ startY: number; startOffset: number } | null>(null);
  // 滚轮步进尺度（窗口高度百分比，1-100，经 DataManager 持久化）
  const [wheelPctRaw, setWheelPct] = useData(store.keys["cover-wheel-pct"]);
  const wheelPct = Math.min(100, Math.max(1, parseInt(wheelPctRaw, 10) || 10));

  const pw = index?.pw ?? 400;
  const windowH = Math.round((pw * 7) / 5); // 预览坐标下裁剪窗口高度 560
  const maxOff = index ? Math.max(0, index.total_ph - windowH) : 0;
  // 显示比例：容器高对应的窗口宽（5:7）÷ 预览宽
  const scale = containerH > 0 ? (containerH * 5) / 7 / pw : 1;
  const cropW = (containerH * 5) / 7;

  const clampOffset = useCallback(
    (v: number) => Math.max(0, Math.min(maxOff, v)),
    [maxOff],
  );

  // 容器高度（决定显示缩放）。容器在 loading 结束后才渲染，需以 loading 为依赖
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const update = () => setContainerH(el.clientHeight);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [loading]);

  // 加载页索引 + 恢复上次裁剪偏移
  useEffect(() => {
    Promise.all([api.getComicPageIndex(comic.id), api.getComicConfig(comic.id)])
      .then(([idx, cfg]) => {
        setIndex(idx);
        const saved = parseInt(cfg.cover_crop_offset ?? "", 10);
        if (!isNaN(saved) && saved >= 0) {
          setOffset(Math.min(saved, Math.max(0, idx.total_ph - Math.round((idx.pw * 7) / 5))));
        }
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [comic.id]);

  // 滚轮移动裁剪窗口（原生监听以 preventDefault），步进为窗口高度 × wheelPct%
  useEffect(() => {
    const el = containerRef.current;
    if (!el || !index) return;
    const step = Math.max(1, Math.round((windowH * wheelPct) / 100));
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setOffset((o) => clampOffset(o + (e.deltaY > 0 ? step : -step)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [index, windowH, wheelPct, clampOffset]);

  // 拖拽移动裁剪窗口
  useEffect(() => {
    const move = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d) return;
      const dy = e.clientY - d.startY;
      setOffset(clampOffset(d.startOffset - Math.round(dy / scale)));
    };
    const up = () => {
      dragRef.current = null;
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
    return () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
    };
  }, [scale, clampOffset]);

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    dragRef.current = { startY: e.clientY, startOffset: offset };
  };

  // 当前窗口附近的可见页范围（前后各 2 页缓冲）
  const visible = index
    ? computeVisible(index, offset)
    : { first: 0, last: 0, cumBefore: 0 };

  // 预取：窗口附近更宽的页提前加载，滚动时不出现空白等待
  useEffect(() => {
    if (!index) return;
    const { first, last } = computeVisible(index, offset);
    const PREFETCH = 4;
    for (let i = Math.max(0, first - PREFETCH); i <= Math.min(index.pages.length - 1, last + PREFETCH); i++) {
      getPreview(comic.id, i).catch(() => {});
    }
  }, [index, offset, comic.id]);

  // 全量预取：打开弹窗后后台并发预取整章全部页面缩略图（含不可见区域），
  // 滚动/拖拽到任意页即时显示，避免滚动到才生成等待。
  // 已缓存的页 getPreview 直接返回，不会重复请求；窗口附近页已被上方 effect 抢先。
  const PREVIEW_CONCURRENCY = 6;
  useEffect(() => {
    if (!index) return;
    let cancelled = false;
    let next = 0;
    const worker = async () => {
      while (!cancelled) {
        const i = next++;
        if (i >= index.pages.length) return;
        try {
          await getPreview(comic.id, i);
        } catch {
          /* 单页失败不阻塞后续页 */
        }
      }
    };
    const workers: Promise<void>[] = [];
    for (let k = 0; k < Math.min(PREVIEW_CONCURRENCY, index.pages.length); k++) {
      workers.push(worker());
    }
    return () => {
      cancelled = true;
    };
  }, [index, comic.id]);

  const save = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.setComicCoverFromOffset(comic.id, offset);
      onPicked();
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const reset = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.resetComicCover(comic.id);
      onPicked();
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const labels = ["顶部", "偏上", "中部", "偏下", "底部"];
  const ratio = maxOff > 0 ? offset / maxOff : 0;
  const label = labels[Math.min(Math.floor(ratio * 5), 4)];

  return (
    <div className="pick-cover-overlay" onClick={onClose}>
      <div className="pick-cover-modal" onClick={(e) => e.stopPropagation()}>
        <div className="pick-cover-header">
          <span className="pick-cover-title" title={comic.title}>
            选择封面：{comic.title}
          </span>
          <div className="pick-cover-actions">
            <button className="btn-ghost" onClick={reset} disabled={busy}>
              恢复默认
            </button>
            <button className="btn-primary" onClick={save} disabled={busy}>
              设为封面
            </button>
            <button className="btn-ghost" onClick={onClose}>
              关闭
            </button>
          </div>
        </div>
        {error && <div className="shelf-error">{error}</div>}
        {loading ? (
          <div className="center-hint">分析章节页面…</div>
        ) : index && index.pages.length > 0 ? (
          <>
            <div
              ref={containerRef}
              className="pick-cover-container"
              onMouseDown={onMouseDown}
            >
              <div
                className="pick-cover-strip"
                style={{
                  top: -(offset - visible.cumBefore) * scale,
                  width: cropW,
                }}
              >
                {index.pages.slice(visible.first, visible.last + 1).map((p, i) => {
                  const pageIdx = visible.first + i;
                  return (
                    <PageStripImg
                      key={pageIdx}
                      comicId={comic.id}
                      pageIdx={pageIdx}
                      height={p.ph * scale}
                    />
                  );
                })}
              </div>
              <div className="pick-cover-window" style={{ width: cropW, height: containerH }}>
                <span className="pick-cover-position">{label}</span>
              </div>
            </div>
            <div className="pick-cover-toolbar">
              <input
                className="pick-cover-slider"
                type="range"
                min={0}
                max={Math.max(1, maxOff)}
                step={1}
                value={offset}
                onChange={(e) => setOffset(clampOffset(parseInt(e.target.value, 10)))}
              />
              <span className="pick-cover-px">
                {offset} / {maxOff} px
              </span>
              <div className="pick-cover-scroll-step">
                <span className="pick-cover-hint">滚动</span>
                <input
                  type="range"
                  min={1}
                  max={100}
                  step={1}
                  value={wheelPct}
                  onChange={(e) => {
                    const v = Math.min(100, Math.max(1, parseInt(e.target.value, 10) || 10));
                    setWheelPct(String(v));
                  }}
                />
                <span className="pick-cover-px">
                  {wheelPct}% ≈ {Math.max(1, Math.round((windowH * wheelPct) / 100))}px
                </span>
              </div>
            </div>
          </>
        ) : (
          <div className="center-hint">该章节无页面</div>
        )}
      </div>
    </div>
  );
}

/** 条带中的单页预览（缓存命中立即显示，未命中请求 400 宽缩略图） */
function PageStripImg({
  comicId,
  pageIdx,
  height,
}: {
  comicId: string;
  pageIdx: number;
  height: number;
}) {
  const [src, setSrc] = useState<string>();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setFailed(false);
    getPreview(comicId, pageIdx)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [comicId, pageIdx]);

  return (
    <div className="pick-cover-page-wrap" style={{ height }}>
      {src && <img src={src} draggable={false} alt="" className="pick-cover-page" />}
      {failed && <span className="pick-cover-page-fail">第 {pageIdx + 1} 页</span>}
    </div>
  );
}

/** 系列章节列表（整卡封面风格，参考旧项目 MangaShelf） */
export default function ChapterView({ series, onBack, onOpenChapter }: Props) {
  const [chapters, setChapters] = useState<Comic[]>([]);
  const [loading, setLoading] = useState(true);
  const [completed, setCompleted] = useState<boolean | null>(null);
  const [error, setError] = useState("");
  // 自定义封面后递增版本号，仅刷新被修改章节的封面
  const [coverVersions, setCoverVersions] = useState<Record<string, number>>({});
  // 正在选封面的章节
  const [picking, setPicking] = useState<Comic | null>(null);
  // 嵌套系列：点击 series 类型的章节时进入下一层章节列表（如 系列→子系列→PDF 章节）
  const [nested, setNested] = useState<Comic | null>(null);
  // 章节网格滚动容器：按系列记忆滚动位置
  const gridRef = useRef<HTMLDivElement>(null);
  usePersistScroll(`series:${series.id}`, gridRef, !loading);

  const load = useCallback(async () => {
    try {
      const [list, completed] = await Promise.all([
        api.getChapters(series.id),
        api.getCompletedStatus(series.id),
      ]);
      setChapters(list);
      setCompleted(completed);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [series.id]);

  useEffect(() => {
    load();
  }, [load]);

  const toggleCompleted = async () => {
    const next = !completed;
    try {
      await api.setCompletedStatus(series.id, next);
      setCompleted(next);
    } catch (e) {
      setError(String(e));
    }
  };

  // 嵌套系列：渲染子章节列表（hooks 全部在顶部，保持调用顺序稳定）
  if (nested) {
    return (
      <ChapterView
        series={nested}
        onBack={() => setNested(null)}
        onOpenChapter={onOpenChapter}
      />
    );
  }

  return (
    <div className="chapter-page">
      <div className="chapter-header">
        <button className="btn-ghost" onClick={onBack}>← 返回</button>
        <h2 className="chapter-title">{series.title}</h2>
        <span className="chapter-sub">{chapters.length} 章</span>
        {completed !== null && (
          <button className="btn-ghost" onClick={toggleCompleted}>
            {completed ? "已完结 ✓" : "标记完结"}
          </button>
        )}
      </div>

      {error && <div className="shelf-error">{error}</div>}

      {loading ? (
        <div className="center-hint">加载中…</div>
      ) : chapters.length === 0 ? (
        <div className="center-hint">暂无章节</div>
      ) : (
        <div className="shelf-grid chapter-grid" ref={gridRef}>
          {chapters.map((ch) => (
            <div
              key={ch.id}
              className="comic-card chapter-card"
              onClick={() => (ch.type === "series" ? setNested(ch) : onOpenChapter(ch))}
            >
              <CoverImage
                key={`${ch.id}-${coverVersions[ch.id] ?? 0}`}
                comicId={ch.id}
                className="cover"
              />
              <button
                className="pick-cover-btn"
                title="设置自定义封面"
                onClick={(e) => {
                  e.stopPropagation();
                  setPicking(ch);
                }}
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z" />
                </svg>
              </button>
              <div className="card-info">
                <div className="card-title" title={ch.title}>{ch.title}</div>
                <div className="card-meta">
                  {ch.type === "series"
                    ? `${ch.chapter_count ?? 0} 章`
                    : `${ch.page_count > 0 ? `${ch.page_count} 页 · ` : ""}${formatSize(ch.size)}`}
                </div>
              </div>
              {ch.type !== "series" && ch.reading_progress > 0 && ch.page_count > 0 && (
                <div className="chapter-progress">
                  <div
                    className="chapter-progress-fill"
                    style={{ width: `${Math.min(100, Math.round((ch.reading_progress / ch.page_count) * 100))}%` }}
                  />
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {picking && picking.format === ".pdf" && (
        <PdfPickCoverModal
          comic={picking}
          onPicked={() =>
            setCoverVersions((v) => ({ ...v, [picking.id]: (v[picking.id] ?? 0) + 1 }))
          }
          onClose={() => setPicking(null)}
        />
      )}
      {picking && picking.format !== ".pdf" && (
        <PickCoverModal
          comic={picking}
          onPicked={() =>
            setCoverVersions((v) => ({ ...v, [picking.id]: (v[picking.id] ?? 0) + 1 }))
          }
          onClose={() => setPicking(null)}
        />
      )}
    </div>
  );
}
