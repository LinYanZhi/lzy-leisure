import { useCallback, useEffect, useRef, useState } from "react";
import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { api, imgUrl, isWeb, type StoryIssue } from "../../api";
import { useIsMobile } from "../../useIsMobile";
import { loadStoryPdf, renderFirstPageDataUrl, tryFirstPageJpeg } from "./pdfCover";

interface Props {
  issue: StoryIssue;
  onClose: () => void;
}

/** 两页之间的书脊间距（px） */
const GUTTER = 48;

/**
 * 故事会双页书式阅读器。
 * 与漫画的上下滚动不同：一次展示两页（左页 + 右页），像翻书一样上一页/下一页。
 * 跨页规则（页码 0 起）：
 *   - spread = 0：封面单页（第一页单独一页）
 *   - spread 为奇数：显示 (spread, spread+1) 两页
 */
export default function StoryReader({ issue, onClose }: Props) {
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [dims, setDims] = useState<{ w: number; h: number }[]>([]);
  const [error, setError] = useState("");
  // 当前跨页的第一页 index（0 起）
  const [spread, setSpread] = useState(0);
  const spreadRef = useRef(0);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 顶栏/底栏显隐：点击空白区域切换（沉浸阅读）
  const [uiHidden, setUiHidden] = useState(false);
  // 切换控件显隐时的短暂提示（自动淡出，不遮挡阅读）
  const [uiHint, setUiHint] = useState(false);
  const hintTimer = useRef(0);
  const toggleUi = () => {
    setUiHidden((v) => !v);
    setUiHint(true);
    window.clearTimeout(hintTimer.current);
    hintTimer.current = window.setTimeout(() => setUiHint(false), 2600);
  };
  useEffect(() => () => window.clearTimeout(hintTimer.current), []);
  // 内容区尺寸（决定页面渲染 scale）
  const stageRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const [stage, setStage] = useState({ w: 0, h: 0 });
  // 浏览器/手机端：逐页图片流阅读（pdf.js Range 流在移动端内核不可靠）
  const imgMode = isWeb;
  // 手机端视口（内容区点击放大/缩小查看，翻页仅底部按钮）
  const isMobile = useIsMobile();
  // 内容区放大状态：点击 toggle；放大后可单指拖拽 + 双指捏合查看，不触发翻页
  const [zoomed, setZoomed] = useState(false);
  const zoomedRef = useRef(false);
  const imgRef = useRef<HTMLImageElement>(null);
  const zoomScale = useRef(2);
  const zoomTx = useRef(0);
  const zoomTy = useRef(0);
  // 本次触摸手势是否有位移（拖拽/捏合后吞掉伴随 click，避免误缩小）
  const touchMoved = useRef(false);
  // 页码跳转列表（底部页数按钮点击弹出）
  const [showIndex, setShowIndex] = useState(false);

  // 测量内容区（工具栏之外的可视区域）
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const update = () => setStage({ w: el.clientWidth, h: el.clientHeight });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [doc]);

  // 加载 PDF（URL 流式，失败回退 base64）→ 解析页比例 → 恢复进度。
  // 浏览器/手机端（isWeb）：pdf.js Range 流在部分移动端内核会中断黑屏，
  // 改走「逐页图片流」（/api/img/story-page/{id}/{idx}），不解析 PDF。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (imgMode) {
        // 页数取扫描回写的 page_count（可能为 0：未知页数时翻到末尾以图片 404 停住）
        const total = issue.page_count > 0 ? issue.page_count : 0;
        let p = issue.reading_progress;
        if (p < 0) p = 0;
        if (total > 0 && p >= total) p = total - 1;
        if (p > 0 && p % 2 === 0) p -= 1; // 桌面书式进度（跨页第一页）→ 手机单页同样适用
        if (cancelled) return;
        spreadRef.current = p;
        setSpread(p);
        return;
      }
      try {
        const pdf = await loadStoryPdf(issue.id);
        if (cancelled) return;
        const d: { w: number; h: number }[] = [];
        for (let n = 1; n <= pdf.numPages; n++) {
          const page = await pdf.getPage(n);
          const vp = page.getViewport({ scale: 1 });
          d.push({ w: vp.width, h: vp.height });
        }
        if (cancelled) return;
        setDims(d);
        setDoc(pdf);
        // 回写真实页数（书架立即补全；幂等）
        api.storyclubUpdatePageCount(issue.id, pdf.numPages).catch(() => {});
        // 恢复进度：reading_progress 存当前跨页第一页；0 = 封面单页
        let p = issue.reading_progress;
        if (p < 0 || p >= pdf.numPages) p = 0;
        if (p > 0 && p % 2 === 0) p -= 1; // 偶数页码不可能为跨页第一页，回退一页
        spreadRef.current = p;
        setSpread(p);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [issue.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // 首次加载后渲染第一页上传为封面（已有封面则静默覆盖为最新内容）
  const coverUploadedRef = useRef(false);
  useEffect(() => {
    if (!doc || coverUploadedRef.current) return;
    coverUploadedRef.current = true;
    (async () => {
      try {
        // 优先后端直提第一页 JPEG（扫描件毫秒级，无需渲染），失败退回 pdf.js 渲染
        const dataUrl =
          (await tryFirstPageJpeg(issue.id)) || (await renderFirstPageDataUrl(doc));
        await api.storyclubUploadCover(issue.id, dataUrl);
      } catch {
        // 封面生成失败静默，书架显示占位
      }
    })();
  }, [doc, issue.id]);

  const numPages = dims.length;
  // 总页数：图片流模式取扫描回写的 page_count（桌面模式为 pdf.js 解析页数）
  const totalPages = imgMode ? (issue.page_count > 0 ? issue.page_count : 0) : numPages;

  // ── 翻页（书式：上一跨页 / 下一跨页；图片流：单页 ±1） ──
  const canPrev = spread > 0;
  const canNext = imgMode
    ? totalPages > 0
      ? spread + 1 < totalPages
      : true // 页数未知（page_count=0）：允许尝试，末尾由图片 404 提示
    : spread === 0
      ? numPages > 1
      : spread + 2 < numPages;

  const scheduleSave = useCallback(
    (p: number) => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        api.storyclubSetProgress(issue.id, p).catch(() => {});
      }, 500);
    },
    [issue.id],
  );

  const goSpread = useCallback(
    (next: number) => {
      if (next === spreadRef.current) return;
      if (next < 0) return;
      if (imgMode) {
        if (totalPages > 0 && next >= totalPages) return;
        spreadRef.current = next;
        setSpread(next);
        scheduleSave(next);
        return;
      }
      if (next === 0 ? numPages <= 0 : next >= numPages) return;
      spreadRef.current = next;
      setSpread(next);
      scheduleSave(next);
    },
    [numPages, scheduleSave, imgMode],
  );

  const nextSpread = useCallback(() => {
    const s = spreadRef.current;
    if (imgMode) {
      goSpread(s + 1);
      return;
    }
    if (s === 0) goSpread(1);
    else goSpread(s + 2);
  }, [goSpread, imgMode]);

  const prevSpread = useCallback(() => {
    const s = spreadRef.current;
    if (imgMode) {
      goSpread(s - 1);
      return;
    }
    if (s <= 1) goSpread(0);
    else goSpread(s - 2);
  }, [goSpread, imgMode]);

  // 快捷键（仅故事会阅读器内）：←/A/W/↑ 上一跨页、→/D/S/↓ 下一跨页、Esc/Q 返回
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      if (e.key === "Escape" || k === "q") {
        if (showIndex) setShowIndex(false);
        else onClose();
        return;
      }
      if (e.key === "ArrowLeft" || k === "a" || k === "w" || e.key === "ArrowUp")
        prevSpread();
      else if (e.key === "ArrowRight" || k === "d" || k === "s" || e.key === "ArrowDown")
        nextSpread();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, prevSpread, nextSpread, showIndex]);

  // 滚轮翻页（书式阅读：上滚上一页 / 下滚下一页）
  // 不做时间节流，改为按滚动量累计：达到一档（约 96px）就翻一页，
  // 快速连滚/触控板惯性滑动可连续翻页，滚动很慢时不误翻。
  const wheelAcc = useRef(0);
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      wheelAcc.current += e.deltaY;
      const THRESHOLD = 96;
      while (Math.abs(wheelAcc.current) >= THRESHOLD) {
        if (wheelAcc.current > 0) nextSpread();
        else prevSpread();
        wheelAcc.current -= Math.sign(wheelAcc.current) * THRESHOLD;
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [prevSpread, nextSpread]);

  // 卸载时保存一次
  useEffect(() => {
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      api.storyclubSetProgress(issue.id, spreadRef.current).catch(() => {});
    };
  }, [issue.id]);

  // ── 页面布局计算 ──
  const layout = (() => {
    if (stage.w <= 0 || stage.h <= 0 || numPages === 0) return null;
    const { w, h } = stage;
    const page = (i: number) => dims[i] ?? null;
    if (spread === 0) {
      const p = page(0);
      if (!p) return null;
      const scale = Math.min(h / p.h, w / p.w);
      return { pages: [{ idx: 0, pw: p.w * scale, ph: p.h * scale }], single: true };
    }
    const left = page(spread);
    const right = page(spread + 1);
    if (!left) return null;
    const maxH = Math.max(left.h, right ? right.h : 0);
    const totalW = left.w + (right ? right.w : 0);
    const scale = Math.min(h / maxH, (w - GUTTER) / totalW);
    const pages = [{ idx: spread, pw: left.w * scale, ph: left.h * scale }];
    if (right) pages.push({ idx: spread + 1, pw: right.w * scale, ph: right.h * scale });
    return { pages, single: false };
  })();

  // 页码指示文案
  const pageLabel = (() => {
    if (imgMode) {
      const total = totalPages > 0 ? ` · 共 ${totalPages} 页` : "";
      return `第 ${spread + 1} 页${total}`;
    }
    if (numPages === 0) return "";
    if (spread === 0) return `封面 · 共 ${numPages} 页`;
    if (spread + 2 <= numPages) return `第 ${spread + 1}-${spread + 2} 页 · 共 ${numPages} 页`;
    return `第 ${spread + 1} 页 · 共 ${numPages} 页`;
  })();

  // 内容区点击 toggle 放大/缩小（手机端：整页看不清字时放大查看）。
  // 放大以点击位置为原点：点击处的内容放大后保持原位（点哪看哪），带平滑过渡。
  const toggleZoom = useCallback((clientX?: number, clientY?: number) => {
    const el = imgRef.current;
    const slot = el?.parentElement;
    if (!el || !slot) return;
    if (zoomedRef.current) {
      // 缩小回整页（回中）
      zoomScale.current = 2;
      zoomTx.current = 0;
      zoomTy.current = 0;
      el.style.transition = "transform 0.25s ease";
      el.style.transform = "translate(0px, 0px) scale(1)";
      setZoomed(false);
      return;
    }
    // 放大：点击处内容保持原位（围绕点击位置放大）
    if (zoomScale.current < 1.1) zoomScale.current = 2;
    const rect = slot.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    const fx = (clientX ?? rect.left + w / 2) - rect.left;
    const fy = (clientY ?? rect.top + h / 2) - rect.top;
    zoomTx.current = w / 2 - fx;
    zoomTy.current = h / 2 - fy;
    el.style.transition = "transform 0.25s ease";
    el.style.transform = `translate(${zoomTx.current}px, ${zoomTy.current}px) scale(${zoomScale.current})`;
    setZoomed(true);
  }, []);

  // 手机端内容区手势：单指拖拽平移 + 双指捏合缩放（不触发翻页）。
  // 需非 passive 监听 touchmove 才能 preventDefault，避免页面跟随滚动/缩放。
  useEffect(() => {
    if (!imgMode) return;
    const el = imgRef.current;
    const stageEl = stageRef.current;
    if (!el || !stageEl) return;
    // 手势中间态（effect 闭包内共享）
    let panStartX = 0;
    let panStartY = 0;
    let panStartTx = 0;
    let panStartTy = 0;
    let pinchStartDist = 0;
    let pinchStartScale = 2;
    let pinchStartTx = 0;
    let pinchStartTy = 0;
    let pinching = false;

    const apply = (animate: boolean) => {
      el.style.transition = animate ? "transform 0.25s ease" : "none";
      el.style.transform = `translate(${zoomTx.current}px, ${zoomTy.current}px) scale(${zoomScale.current})`;
    };

    // 平移边界：内容中心最多拖到视口边缘（略留余量）
    const clamp = () => {
      const w = stageEl.clientWidth || window.innerWidth;
      const h = stageEl.clientHeight || window.innerHeight;
      const s = zoomScale.current;
      const mx = (w * (s - 1)) / 2 + 40;
      const my = (h * (s - 1)) / 2 + 40;
      zoomTx.current = Math.min(mx, Math.max(-mx, zoomTx.current));
      zoomTy.current = Math.min(my, Math.max(-my, zoomTy.current));
    };

    const dist = (t: TouchList) =>
      Math.hypot(t[0].clientX - t[1].clientX, t[0].clientY - t[1].clientY);

    const onStart = (e: TouchEvent) => {
      touchMoved.current = false;
      pinching = e.touches.length >= 2;
      if (pinching) {
        pinchStartDist = dist(e.touches);
        pinchStartScale = zoomScale.current;
        pinchStartTx = zoomTx.current;
        pinchStartTy = zoomTy.current;
      } else {
        panStartX = e.touches[0].clientX;
        panStartY = e.touches[0].clientY;
        panStartTx = zoomTx.current;
        panStartTy = zoomTy.current;
      }
    };

    const onMove = (e: TouchEvent) => {
      e.preventDefault();
      if (e.touches.length >= 2) {
        pinching = true;
        const d = dist(e.touches);
        if (pinchStartDist > 0) {
          const s = Math.min(6, Math.max(1, pinchStartScale * (d / pinchStartDist)));
          zoomScale.current = s;
          // 捏合直接放大/缩小：同步放大状态（避免后续拖拽/点击行为不一致）
          if (s > 1.05 && !zoomedRef.current) setZoomed(true);
          if (
            s <= 1.02 &&
            zoomedRef.current &&
            Math.abs(zoomTx.current) < 2 &&
            Math.abs(zoomTy.current) < 2
          ) {
            setZoomed(false);
          }
          // 保持双指中心下的内容点不动（跟手）
          const cx = (e.touches[0].clientX + e.touches[1].clientX) / 2;
          const cy = (e.touches[0].clientY + e.touches[1].clientY) / 2;
          const rect = stageEl.getBoundingClientRect();
          const fx = cx - rect.left - rect.width / 2;
          const fy = cy - rect.top - rect.height / 2;
          const k = s / pinchStartScale;
          zoomTx.current = fx - k * (fx - pinchStartTx);
          zoomTy.current = fy - k * (fy - pinchStartTy);
          clamp();
        }
        touchMoved.current = true;
      } else if (pinching) {
        // 双指变单指：以当前为平移起点
        pinching = false;
        panStartX = e.touches[0].clientX;
        panStartY = e.touches[0].clientY;
        panStartTx = zoomTx.current;
        panStartTy = zoomTy.current;
      } else {
        const x = e.touches[0].clientX;
        const y = e.touches[0].clientY;
        if (Math.abs(x - panStartX) > 8 || Math.abs(y - panStartY) > 8) {
          touchMoved.current = true;
        }
        if (zoomedRef.current) {
          zoomTx.current = panStartTx + (x - panStartX);
          zoomTy.current = panStartTy + (y - panStartY);
          clamp();
        }
      }
      apply(false);
    };

    const onEnd = () => {
      // 手势结束后短暂保持 touchMoved，吞掉随之而来的 click（避免误缩小）
      setTimeout(() => {
        touchMoved.current = false;
      }, 0);
    };

    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd);
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
    };
  }, [imgMode, stage.w, stage.h]);

  // zoomed 状态同步到 ref（手势闭包读取最新放大态）
  useEffect(() => {
    zoomedRef.current = zoomed;
  }, [zoomed]);

  // 点击内容区：手机端 toggle 放大/缩小（以点击位置为原点）；PC 端保留点左/右翻页
  const onPageClick = (e: React.MouseEvent) => {
    if (touchMoved.current) return; // 拖拽/捏合手势已结束，吞掉伴随 click
    if (isMobile && imgMode) {
      toggleZoom(e.clientX, e.clientY);
      return;
    }
    const el = stageRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (e.clientX < rect.left + rect.width / 2) prevSpread();
    else nextSpread();
  };

  return (
    <div className="reader-overlay story-reader" ref={rootRef}>
      <div className={`reader-toolbar${uiHidden ? " hidden" : ""}`}>
        <button className="btn-ghost" onClick={onClose}>← 返回</button>
        <div className="reader-title" title={issue.title}>
          {issue.title}
        </div>
        <div className="reader-meta">{imgMode ? pageLabel : doc ? pageLabel : "加载中…"}</div>
      </div>

      {error && <div className="shelf-error">{error}</div>}
      {!imgMode && !doc && !error && <div className="center-hint">正在加载 PDF…</div>}

      {/* 空白区域（页面间隙/四周留白）点击切换顶栏底栏显隐；页面区域点击翻页 */}
      <div
        className="story-stage"
        ref={stageRef}
        onClick={toggleUi}
      >
        {uiHint && (
          <div className="story-ui-hint">
            {uiHidden ? "已隐藏控件，点击空白恢复" : "点击空白隐藏控件"}
          </div>
        )}
        {imgMode ? (
          <div
            className="story-page-slot story-page-slot-img"
            onClick={(e) => {
              e.stopPropagation();
              onPageClick(e);
            }}
          >
            <img
              ref={imgRef}
              className="story-page-img"
              src={imgUrl(["story-page", issue.id, String(spread)])}
              alt={`第 ${spread + 1} 页`}
              draggable={false}
              onError={() => {
                // 页数未知（page_count=0）翻过头或该页提取失败：回退一页
                if (numPages === 0 && spread >= 1) {
                  spreadRef.current = spread - 1;
                  setSpread(spread - 1);
                }
              }}
            />
          </div>
        ) : (
          layout &&
          layout.pages.map((p) => (
            <div
              key={p.idx}
              className="story-page-slot"
              onClick={(e) => {
                e.stopPropagation();
                onPageClick(e);
              }}
            >
              <StoryPageCanvas
                doc={doc!}
                pageNum={p.idx + 1}
                width={p.pw}
                height={p.ph}
              />
            </div>
          ))
        )}
      </div>

      {(imgMode || (doc && numPages > 0)) && (
        <div className={`story-controls${uiHidden ? " hidden" : ""}`}>
          <button className="reader-nav-btn" onClick={prevSpread} disabled={!canPrev}>
            上一页
          </button>
          <button
            className="reader-nav-btn story-page-btn"
            onClick={() => {
              if (totalPages > 0) setShowIndex(true);
            }}
            disabled={totalPages <= 0}
          >
            {spread + 1} / {totalPages || "?"}
          </button>
          <span className="story-controls-hint">← A 上一页 · → D 下一页 · Q 返回 · 点击空白隐藏控件</span>
          <button className="reader-nav-btn" onClick={nextSpread} disabled={!canNext}>
            下一页
          </button>
        </div>
      )}

      {/* 页码跳转列表（底部页数按钮点击弹出） */}
      {showIndex && totalPages > 0 && (
        <div
          className="story-index-overlay"
          onClick={(e) => {
            e.stopPropagation();
            setShowIndex(false);
          }}
        >
          <div
            className="story-index-panel"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="story-index-title">
              目录 · 共 {totalPages} 页
            </div>
            <div className="story-index-grid">
              {Array.from({ length: totalPages }, (_, i) => (
                <button
                  key={i}
                  className={`story-index-item${i === spread ? " cur" : ""}`}
                  onClick={() => {
                    goSpread(i);
                    setShowIndex(false);
                  }}
                >
                  {i + 1}
                </button>
              ))}
            </div>
            <button
              className="story-index-close"
              onClick={() => setShowIndex(false)}
            >
              关闭
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** 单页 PDF 渲染（双页书式：宽度/高度由布局计算，按设备像素比清晰渲染） */
function StoryPageCanvas({
  doc,
  pageNum,
  width,
  height,
}: {
  doc: PDFDocumentProxy;
  pageNum: number;
  width: number;
  height: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let cancelled = false;
    let task: RenderTask | null = null;
    (async () => {
      try {
        const page = await doc.getPage(pageNum);
        if (cancelled) return;
        const dpr = window.devicePixelRatio || 1;
        canvas.width = Math.max(1, Math.floor(width * dpr));
        canvas.height = Math.max(1, Math.floor(height * dpr));
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        const base = page.getViewport({ scale: 1 });
        const vp = page.getViewport({ scale: (width / base.width) * dpr });
        task = page.render({ canvas, viewport: vp });
        await task.promise;
      } catch {
        // 单页渲染失败保持空白
      }
    })();
    return () => {
      cancelled = true;
      if (task) task.cancel();
    };
  }, [doc, pageNum, width, height]);

  return (
    <canvas
      ref={canvasRef}
      className="story-page-canvas"
      style={{ width, height }}
    />
  );
}
