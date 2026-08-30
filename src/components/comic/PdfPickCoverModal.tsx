import { useCallback, useEffect, useRef, useState } from "react";
import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy, type RenderTask } from "pdfjs-dist";
// legacy 版 worker 内置 Uint8Array.prototype.toHex 等较新 API 的 polyfill，
// 旧版手机浏览器（Chromium <132 / Safari <18.4 无原生 toHex）也能正常解析 PDF。
import pdfWorkerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { useData } from "@glbt/appkit-ui";
import { api, isWeb, type Comic } from "../../api";
import { store } from "../../data";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

interface Props {
  comic: Comic;
  onPicked: () => void;
  onClose: () => void;
}

/** PDF 选封面弹窗：pdf.js 渲染当前页 + 5:7 裁剪窗口（拖拽/滚轮定位，翻页可选页），截取后上传为封面 */
export default function PdfPickCoverModal({ comic, onPicked, onClose }: Props) {
  // 逻辑渲染宽（与图片版预览基准一致，截图坐标即为该尺寸）
  const PREVIEW_W = 400;
  const windowH = Math.round((PREVIEW_W * 7) / 5); // 裁剪窗口高 560

  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [pageNo, setPageNo] = useState(1);
  const [pageH, setPageH] = useState(0); // 当前页逻辑渲染高
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [containerH, setContainerH] = useState(0);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ startY: number; startOffset: number } | null>(null);
  const renderTaskRef = useRef<RenderTask | null>(null);
  // 每页裁剪偏移记忆（本次会话内）
  const pageOffsetsRef = useRef<Map<number, number>>(new Map());
  // 滚轮步进尺度（窗口高度百分比，1-100，经 DataManager 持久化）
  const [wheelPctRaw, setWheelPct] = useData(store.keys["cover-wheel-pct"]);
  const wheelPct = Math.min(100, Math.max(1, parseInt(wheelPctRaw, 10) || 10));

  const maxOff = Math.max(0, pageH - windowH);
  const clampOffset = useCallback(
    (v: number) => Math.max(0, Math.min(maxOff, v)),
    [maxOff],
  );
  // 显示比例：窗口高（=容器高）对应逻辑宽 400
  const scale = containerH > 0 ? (containerH * 5) / 7 / PREVIEW_W : 1;
  const cropW = (containerH * 5) / 7;

  // 容器高度（决定显示缩放）
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const update = () => setContainerH(el.clientHeight);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [loading]);

  // 加载 PDF + 恢复上次页码/偏移（浏览器模式按 URL Range 流式拉取，桌面取 base64）
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const src = await api.getComicPdfData(comic.id);
        let pdf: PDFDocumentProxy;
        if (isWeb) {
          pdf = await getDocument({ url: src }).promise;
        } else {
          const bin = atob(src);
          const data = new Uint8Array(bin.length);
          for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i);
          pdf = await getDocument({ data }).promise;
        }
        if (cancelled) return;
        const cfg = await api.getComicConfig(comic.id);
        if (cancelled) return;
        const p = Math.min(pdf.numPages, Math.max(1, parseInt(cfg.cover_pdf_page ?? "1", 10) || 1));
        const saved = parseInt(cfg.cover_pdf_offset ?? "0", 10) || 0;
        pageOffsetsRef.current.set(p, saved);
        setDoc(pdf);
        setPageNo(p);
        setOffset(saved); // 页高就绪后由渲染流程收敛到合法范围
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
      if (renderTaskRef.current) renderTaskRef.current.cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [comic.id]);

  // 渲染当前页（宽固定 400，高按页面比例）
  useEffect(() => {
    if (!doc) return;
    let cancelled = false;
    (async () => {
      try {
        const page = await doc.getPage(pageNo);
        if (cancelled) return;
        const base = page.getViewport({ scale: 1 });
        const vp = page.getViewport({ scale: PREVIEW_W / base.width });
        const h = Math.max(1, Math.floor(vp.height));
        if (renderTaskRef.current) renderTaskRef.current.cancel();
        const canvas = canvasRef.current;
        if (!canvas) return;
        canvas.width = PREVIEW_W;
        canvas.height = h;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        const task = page.render({ canvas, viewport: vp });
        renderTaskRef.current = task;
        await task.promise;
        if (cancelled) return;
        setPageH(h);
        // 页高就绪后收敛记忆偏移到合法范围
        setOffset((o) => Math.min(o, Math.max(0, h - windowH)));
      } catch {
        // 渲染取消/失败保持当前页
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [doc, pageNo]);

  // 翻页（记忆当前页偏移，恢复目标页偏移）
  const goPage = useCallback(
    (n: number) => {
      if (!doc) return;
      const next = Math.max(1, Math.min(doc.numPages, n));
      if (next === pageNo) return;
      pageOffsetsRef.current.set(pageNo, offset);
      setPageNo(next);
      setOffset(pageOffsetsRef.current.get(next) ?? 0);
    },
    [doc, pageNo, offset],
  );

  // 滚轮微调裁剪窗口
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const step = Math.max(1, Math.round((windowH * wheelPct) / 100));
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setOffset((o) => clampOffset(o + (e.deltaY > 0 ? step : -step)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [windowH, wheelPct, clampOffset]);

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

  // 截取当前页 [offset, offset+窗口高) 区域为 5:7 封面（超出页面部分黑底补齐）
  const save = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const canvas = canvasRef.current;
      if (!canvas || canvas.height === 0) throw new Error("页面尚未渲染完成");
      const out = document.createElement("canvas");
      out.width = PREVIEW_W;
      out.height = windowH;
      const octx = out.getContext("2d");
      if (!octx) throw new Error("无法创建输出画布");
      octx.fillStyle = "#000";
      octx.fillRect(0, 0, PREVIEW_W, windowH);
      const srcH = Math.max(0, Math.min(windowH, canvas.height - offset));
      if (srcH > 0) octx.drawImage(canvas, 0, offset, PREVIEW_W, srcH, 0, 0, PREVIEW_W, srcH);
      const dataUrl = out.toDataURL("image/jpeg", 0.85);
      await api.uploadComicCover(comic.id, dataUrl, true);
      await api.setComicConfig(comic.id, {
        cover_pdf_page: String(pageNo),
        cover_pdf_offset: String(offset),
      });
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
          <div className="center-hint">加载 PDF…</div>
        ) : doc ? (
          <>
            <div ref={containerRef} className="pick-cover-container" onMouseDown={onMouseDown}>
              <div className="pick-cover-strip" style={{ top: -offset * scale, width: cropW }}>
                <canvas
                  ref={canvasRef}
                  className="pick-cover-page"
                  style={{ width: cropW, height: Math.max(1, pageH * scale) }}
                />
              </div>
              <div className="pick-cover-window" style={{ width: cropW, height: containerH }}>
                <span className="pick-cover-position">{label}</span>
              </div>
            </div>
            <div className="pick-cover-toolbar">
              <div className="pdf-cover-nav">
                <button className="btn-ghost" onClick={() => goPage(pageNo - 1)} disabled={pageNo <= 1}>
                  ←
                </button>
                <input
                  type="number"
                  min={1}
                  max={doc.numPages}
                  value={pageNo}
                  onChange={(e) => goPage(parseInt(e.target.value, 10) || 1)}
                />
                <span className="pick-cover-px">/ {doc.numPages} 页</span>
                <button className="btn-ghost" onClick={() => goPage(pageNo + 1)} disabled={pageNo >= doc.numPages}>
                  →
                </button>
              </div>
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
          <div className="center-hint">PDF 加载失败</div>
        )}
      </div>
    </div>
  );
}
