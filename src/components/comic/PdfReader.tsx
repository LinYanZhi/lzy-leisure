import { useCallback, useEffect, useRef, useState } from "react";
import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy, type RenderTask } from "pdfjs-dist";
// legacy 版 worker 内置 Uint8Array.prototype.toHex 等较新 API 的 polyfill，
// 旧版手机浏览器（Chromium <132 / Safari <18.4 无原生 toHex）也能正常解析 PDF。
import pdfWorkerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { api, isWeb, type Comic } from "../../api";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

interface Props {
  comic: Comic;
  onClose: () => void;
}

/** PDF 连续滚动阅读器（pdf.js 渲染；滚动窗口附近页按需渲染，移出释放内存） */
export default function PdfReader({ comic, onClose }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const pageRefs = useRef<(HTMLDivElement | null)[]>([]);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const currentPageRef = useRef(0);
  const restoredRef = useRef(false);
  const initialPctRef = useRef<number | null>(null);

  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [dims, setDims] = useState<{ w: number; h: number }[]>([]);
  const [width, setWidth] = useState(0);
  const [currentPage, setCurrentPage] = useState(0);
  const [progressPct, setProgressPct] = useState(0);
  const [error, setError] = useState("");

  // 容器宽度（决定渲染 scale）
  const measure = useCallback(() => {
    const el = containerRef.current;
    if (el && el.clientWidth > 0) setWidth(el.clientWidth);
  }, []);
  useEffect(() => {
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [measure]);

  // 加载 PDF：浏览器模式按 URL（Range 分块）流式拉取；桌面模式取 base64 转字节
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
        const d: { w: number; h: number }[] = [];
        for (let n = 1; n <= pdf.numPages; n++) {
          const page = await pdf.getPage(n);
          const vp = page.getViewport({ scale: 1 });
          d.push({ w: vp.width, h: vp.height });
        }
        if (cancelled) return;
        const config = await api.getComicConfig(comic.id);
        if (cancelled) return;
        setDims(d);
        setDoc(pdf);
        const pct = Number(config["scroll_pct"] ?? 0);
        if (pct > 0) initialPctRef.current = Math.min(1, pct / 1000);
        restoredRef.current = true;
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [comic.id]);

  // 占位渲染完成后恢复滚动位置
  useEffect(() => {
    if (dims.length === 0 || !restoredRef.current) return;
    restoredRef.current = false;
    const c = containerRef.current;
    if (!c) return;
    if (initialPctRef.current != null) {
      c.scrollTop = (c.scrollHeight - c.clientHeight) * initialPctRef.current;
    }
  }, [dims.length]);

  // 首次加载后渲染第一页上传为封面（已有封面则后端跳过）
  const coverUploadedRef = useRef(false);
  useEffect(() => {
    if (!doc || coverUploadedRef.current) return;
    coverUploadedRef.current = true;
    (async () => {
      try {
        const page = await doc.getPage(1);
        const vp = page.getViewport({ scale: 2 });
        const canvas = document.createElement("canvas");
        canvas.width = Math.floor(vp.width);
        canvas.height = Math.floor(vp.height);
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        await page.render({ canvas, viewport: vp }).promise;
        const dataUrl = canvas.toDataURL("image/jpeg", 0.85);
        await api.uploadComicCover(comic.id, dataUrl);
      } catch {
        // 封面生成失败静默，占位封面
      }
    })();
  }, [doc, comic.id]);

  const scheduleSave = useCallback(
    (page: number, pct: number) => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        api.setReadingProgress(comic.id, page).catch(() => {});
        api.setComicConfig(comic.id, { scroll_pct: String(Math.round(pct * 1000)) }).catch(() => {});
      }, 500);
    },
    [comic.id],
  );

  const handleScroll = useCallback(() => {
    const c = containerRef.current;
    if (!c) return;
    const mid = c.scrollTop + c.clientHeight / 2;
    let cur = 0;
    const refs = pageRefs.current;
    for (let i = 0; i < refs.length; i++) {
      const el = refs[i];
      if (el && el.offsetTop <= mid) cur = i;
      else break;
    }
    currentPageRef.current = cur;
    setCurrentPage(cur);
    const pct = c.scrollHeight > c.clientHeight ? c.scrollTop / (c.scrollHeight - c.clientHeight) : 1;
    setProgressPct(pct);
    scheduleSave(cur, pct);
  }, [scheduleSave]);

  // ESC 返回
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // 卸载时保存一次
  useEffect(() => {
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      const c = containerRef.current;
      if (c) {
        api.setReadingProgress(comic.id, currentPageRef.current).catch(() => {});
      }
    };
  }, [comic.id]);

  const scale = dims.length > 0 && dims[0].w > 0 && width > 0 ? width / dims[0].w : 1;
  // 渲染窗口：当前页前后各 2 页
  const activeFirst = Math.max(0, currentPage - 2);
  const activeLast = Math.min(dims.length - 1, currentPage + 2);

  return (
    <div className="reader-overlay">
      <div className="reader-toolbar">
        <button className="btn-ghost" onClick={onClose}>← 返回</button>
        <div className="reader-title" title={comic.title}>
          {comic.title}
        </div>
        <div className="reader-meta">
          {doc ? `${currentPage + 1} / ${dims.length} · ${Math.round(progressPct * 100)}%` : "加载中…"}
        </div>
      </div>

      <div className="reader-scroll" ref={containerRef} onScroll={handleScroll}>
        {!doc && !error && <div className="center-hint">正在加载 PDF…</div>}
        {error && <div className="shelf-error">{error}</div>}
        {doc &&
          dims.map((d, i) => (
            <div
              key={i}
              className="reader-page"
              ref={(el) => {
                pageRefs.current[i] = el;
              }}
              style={{ height: Math.round(d.h * scale) }}
            >
              <PdfPageCanvas
                doc={doc}
                pageNum={i + 1}
                width={width}
                active={i >= activeFirst && i <= activeLast}
              />
            </div>
          ))}
        {doc && (
          <div className="reader-end" onClick={onClose}>
            已到末尾，点击返回书架
          </div>
        )}
      </div>
    </div>
  );
}

/** 单页 PDF 渲染（active 时渲染，失活时释放画布内存） */
function PdfPageCanvas({
  doc,
  pageNum,
  width,
  active,
}: {
  doc: PDFDocumentProxy;
  pageNum: number;
  width: number;
  active: boolean;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [rendered, setRendered] = useState(false);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let cancelled = false;
    if (!active) {
      // 移出渲染窗口：清空画布释放内存
      setRendered(false);
      canvas.width = 0;
      return () => {
        cancelled = true;
      };
    }
    let task: RenderTask | null = null;
    (async () => {
      try {
        const page = await doc.getPage(pageNum);
        if (cancelled) return;
        const dpr = window.devicePixelRatio || 1;
        const base = page.getViewport({ scale: 1 });
        const vp = page.getViewport({ scale: (width / base.width) * dpr });
        canvas.width = Math.floor(vp.width);
        canvas.height = Math.floor(vp.height);
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        task = page.render({ canvas, viewport: vp });
        await task.promise;
        if (!cancelled) setRendered(true);
      } catch {
        // 单页渲染失败保持占位
      }
    })();
    return () => {
      cancelled = true;
      if (task) task.cancel();
    };
  }, [doc, pageNum, width, active]);

  return (
    <>
      <canvas
        ref={canvasRef}
        className={rendered ? "reader-pdf-canvas" : "reader-pdf-canvas hidden"}
        style={{ width: width }}
      />
      {!rendered && <div className="reader-page-placeholder" />}
    </>
  );
}
