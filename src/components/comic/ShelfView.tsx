import { useCallback, useEffect, useRef, useState } from "react";
import { useData } from "@glbt/appkit-ui";
import { api, onScanDone, type Comic, type RootDir, type SeriesProgress } from "../../api";
import CoverImage from "./CoverImage";
import { usePersistScroll } from "../../usePersistScroll";
import { store } from "../../data";

interface Props {
  onOpenSeries: (comic: Comic) => void;
  onOpenComic: (comic: Comic) => void;
}

function formatSize(bytes: number): string {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} GB`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** 系列卡片底部徽标文案 */
function seriesBadge(progress: SeriesProgress | undefined): string {
  if (!progress) return "";
  if (progress.all_completed) return "已读完";
  const ch = progress.chapters.find((c) => c.id === progress.continue_chapter_id);
  if (ch) return `续读 ${ch.title} · ${ch.pct}%`;
  return "";
}

/** 漫画完整磁盘路径（root_dir + 相对路径） */
function comicFullPath(c: Comic): string {
  if (c.path === "." || c.path === "") return c.root_dir;
  return `${c.root_dir.replace(/[\\/]+$/, "")}\\${c.path.replace(/^[\\/]+/, "")}`;
}

/** 漫画书架主页：漫画源管理（弹窗）+ 漫画网格 */
export default function ShelfView({ onOpenSeries, onOpenComic }: Props) {
  const [rootDirs, setRootDirs] = useState<RootDir[]>([]);
  const [comics, setComics] = useState<Comic[]>([]);
  const [seriesProgress, setSeriesProgress] = useState<Record<string, SeriesProgress>>({});
  const [completedMap, setCompletedMap] = useState<Record<string, boolean>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  // 当前选中的导入路径（左上角目录标签切换；useData 自动持久化）
  const [activeRoot, setActiveRoot] = useData(store.keys["comic-active-root"]);
  // 漫画源管理弹窗
  const [showManager, setShowManager] = useState(false);
  const [managerError, setManagerError] = useState("");
  // 行级忙碌状态：root:<path> / comic:<id>
  const [busy, setBusy] = useState<string | null>(null);
  // 拖拽排序：当前拖拽卡片的索引（实时重排后位置会变）
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  // 网格滚动容器：按导入路径记忆滚动位置
  const gridRef = useRef<HTMLDivElement>(null);
  usePersistScroll(`shelf:${activeRoot}`, gridRef, rootDirs.length > 0);

  const refresh = useCallback(async () => {
    try {
      const [dirs, list, progress, completed] = await Promise.all([
        api.getRootDirs(),
        api.getTopLevelComics(),
        api.getAllSeriesProgress(),
        api.getAllCompletedStatus(),
      ]);
      setRootDirs(dirs);
      setComics(list);
      setSeriesProgress(progress);
      setCompletedMap(completed);
      if (!dirs.some((d) => d.path === activeRoot)) {
        setActiveRoot(dirs[0]?.path ?? "");
      }
    } catch (e) {
      setError(String(e));
    }
  }, [activeRoot]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    const unlisten = onScanDone(() => {
      setLoading(false);
      refresh();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refresh]);

  // ── 漫画源管理弹窗 ──

  const openManager = () => {
    setManagerError("");
    setShowManager(true);
    refresh();
  };

  const addRoot = async () => {
    const dir = await api.pickComicDir();
    if (!dir) return;
    setLoading(true);
    setError("");
    setManagerError("");
    try {
      await api.addRootDir(dir);
      await api.scanRootDir(dir);
    } catch (e) {
      setError(String(e));
      setManagerError(String(e));
      setLoading(false);
    }
  };

  /** 重新扫描导入路径（以该路径为根整体重扫） */
  const rescanRoot = async (path: string) => {
    const key = `root:${path}`;
    setBusy(key);
    setManagerError("");
    try {
      await api.scanRootDir(path);
    } catch (e) {
      setManagerError(String(e));
    } finally {
      setBusy(null);
    }
  };

  /** 移除导入路径（记录 + 可再生的应用库缓存；保留目录外部封面数据） */
  const removeRoot = async (path: string) => {
    if (!confirm(`确定移除根目录「${path}」及其全部记录？重新添加后可再次扫描导入。`)) return;
    setManagerError("");
    try {
      await api.removeRootDir(path);
      refresh();
    } catch (e) {
      setManagerError(String(e));
    }
  };

  /** 重新扫描单个漫画（只更新该漫画，不动同根其他漫画） */
  const rescanComic = async (id: string) => {
    const key = `comic:${id}`;
    setBusy(key);
    setManagerError("");
    try {
      await api.rescanComic(id);
    } catch (e) {
      setManagerError(String(e));
    } finally {
      setBusy(null);
    }
  };

  /** 移除单个漫画（从书架移除记录；重扫所在根目录会重新导入） */
  const removeComic = async (c: Comic) => {
    if (!confirm(`确定从书架移除「${c.title}」？重新扫描其所在目录会重新导入。`)) return;
    setManagerError("");
    try {
      await api.deleteComic(c.id);
      refresh();
    } catch (e) {
      setManagerError(String(e));
    }
  };

  const openDir = async (comic: Comic) => {
    try {
      await api.openComicDirectory(comic.id);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleCardClick = (c: Comic) => {
    // 拖拽结束后浏览器仍会补发一次 click，这里跳过避免误进入
    if (suppressClickRef.current) {
      suppressClickRef.current = false;
      return;
    }
    // 系列 → 章节列表页；单本 → 直接阅读
    if (c.type === "series") {
      onOpenSeries(c);
      return;
    }
    onOpenComic(c);
  };

  const visibleComics = activeRoot
    ? comics.filter((c) => c.root_dir === activeRoot)
    : comics;

  // 用 ref 保存最新值，避免 pointer 监听闭包过期
  const visibleRef = useRef(visibleComics);
  visibleRef.current = visibleComics;

  // 将新顺序的可见列表合并回 comics（保留其他根目录的相对位置）
  const applyVisibleOrder = (next: Comic[]) => {
    const vis = visibleRef.current;
    setComics((prev) => {
      const visibleIds = new Set(vis.map((c) => c.id));
      const out: Comic[] = [];
      let vi = 0;
      for (const c of prev) {
        if (visibleIds.has(c.id)) out.push(next[vi++] ?? c);
        else out.push(c);
      }
      return out;
    });
  };
  const applyOrderRef = useRef(applyVisibleOrder);
  applyOrderRef.current = applyVisibleOrder;

  // ── Pointer 事件拖拽排序（绕开 WebView2 对 img 区域的 HTML5 DnD 兼容问题）──
  // 触屏设备（手机/平板）无拖拽体验，直接禁用
  const isTouch =
    typeof window !== "undefined" && window.matchMedia("(pointer: coarse)").matches;
  const dragStateRef = useRef<{ id: string; startX: number; startY: number; active: boolean } | null>(null);
  const suppressClickRef = useRef(false);

  // 卡片按下：记录起点（暂不阻止，保持点击打开正常）
  const onCardPointerDown = (e: React.PointerEvent, idx: number) => {
    if (e.button !== 0 || isTouch) return;
    dragStateRef.current = {
      id: visibleComics[idx].id,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
    };
  };

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = dragStateRef.current;
      if (!d) return;
      // 超过阈值才进入拖拽（避免与普通点击冲突）
      if (!d.active) {
        if (Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < 6) return;
        d.active = true;
        suppressClickRef.current = true;
        setDragIndex(visibleRef.current.findIndex((c) => c.id === d.id));
      }
      // 找到鼠标下方卡片，实时重排
      const cardEl = document.elementFromPoint(e.clientX, e.clientY);
      const card = (cardEl instanceof HTMLElement ? cardEl.closest(".comic-card") : null) as HTMLElement | null;
      const to = card ? Number(card.dataset.idx) : NaN;
      if (!Number.isInteger(to)) return;
      setDragIndex((from) => {
        if (from === null || from === to) return from;
        const vis = visibleRef.current;
        const next = [...vis];
        const [moved] = next.splice(from, 1);
        next.splice(to, 0, moved);
        applyOrderRef.current(next);
        return to;
      });
    };
    const onUp = () => {
      const d = dragStateRef.current;
      if (d?.active) {
        api
          .batchSetSortOrder(visibleRef.current.map((c, i) => [i + 1, c.id]))
          .catch((e) => setError(String(e)));
      }
      dragStateRef.current = null;
      setDragIndex(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, []);

  return (
    <div className="shelf">
      <div className="shelf-toolbar">
        <div className="shelf-roots">
          {rootDirs.map((d) => (
            <div
              key={d.path}
              className={`root-chip${d.path === activeRoot ? " active" : ""}`}
              onClick={() => setActiveRoot(d.path)}
              title={d.path}
            >
              <span className="root-name">{d.name}</span>
              <span className="root-count">{d.comic_count}</span>
            </div>
          ))}
        </div>
        <div className="shelf-toolbar-actions">
          <button className="btn-ghost" onClick={openManager} title="管理导入的漫画路径">
            漫画源管理
          </button>
        </div>
      </div>

      {error && <div className="shelf-error">{error}</div>}

      {rootDirs.length === 0 && !loading ? (
        <div className="shelf-empty">
          <p>书架为空</p>
          <p className="muted">点击右上角「漫画源管理」，添加存放漫画的文件夹</p>
        </div>
      ) : (
        <div className="shelf-grid" ref={gridRef}>
          {visibleComics.map((c, idx) => {
            const badge =
              c.type === "series"
                ? seriesBadge(seriesProgress[c.id])
                : c.reading_progress > 0
                  ? `读到 ${c.reading_progress} / ${c.page_count} 页`
                  : "";
            const completed = c.type === "series" && completedMap[c.id];
            return (
              <div
                key={c.id}
                className={`comic-card${dragIndex === idx ? " dragging" : ""}`}
                data-idx={idx}
                title={isTouch ? "" : "拖拽可调整排序"}
                onClick={() => handleCardClick(c)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  openDir(c);
                }}
                onPointerDown={(e) => onCardPointerDown(e, idx)}
              >
                <div className="comic-cover-wrap">
                  <CoverImage comicId={c.id} />
                  <button
                    className="comic-open-btn"
                    title="打开所在目录"
                    onClick={(e) => {
                      e.stopPropagation();
                      openDir(c);
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                    </svg>
                  </button>
                  {completed && <span className="comic-badge comic-badge-completed">完结</span>}
                  {badge && <div className="comic-progress">{badge}</div>}
                </div>
                <div className="comic-title" title={c.title}>
                  {c.title}
                </div>
                <div className="comic-meta">
                  {c.type === "series"
                    ? `${c.chapter_count} 章 · ${formatSize(c.size)}`
                    : `${c.page_count} 页 · ${formatSize(c.size)}`}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {showManager && (
        <div
          className="source-overlay"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowManager(false);
          }}
        >
          <div className="source-modal">
            <div className="source-header">
              <span className="source-title">漫画源管理</span>
              <button className="source-close" onClick={() => setShowManager(false)} title="关闭">
                ✕
              </button>
            </div>
            <div className="source-body">
              {managerError && <div className="shelf-error">{managerError}</div>}

              <div className="source-section-title">导入的路径</div>
              {rootDirs.length === 0 ? (
                <div className="source-empty">暂无导入路径</div>
              ) : (
                <div className="source-list">
                  {rootDirs.map((d) => (
                    <div className="source-row" key={d.path}>
                      <span className="source-path" title={d.path}>
                        {d.path}
                      </span>
                      <span className="source-actions">
                        <button
                          className="btn-ghost"
                          disabled={busy === `root:${d.path}`}
                          onClick={() => rescanRoot(d.path)}
                        >
                          {busy === `root:${d.path}` ? "扫描中…" : "重新扫描"}
                        </button>
                        <button className="btn-danger" onClick={() => removeRoot(d.path)}>
                          移除
                        </button>
                      </span>
                    </div>
                  ))}
                </div>
              )}

              <div className="source-section-title">检测出的漫画</div>
              {comics.length === 0 ? (
                <div className="source-empty">暂无检测到的漫画</div>
              ) : (
                <div className="source-list">
                  {comics.map((c) => (
                    <div className="source-row" key={c.id}>
                      <span className="source-path" title={comicFullPath(c)}>
                        {comicFullPath(c)}
                      </span>
                      <span className="source-actions">
                        <button
                          className="btn-ghost"
                          disabled={busy === `comic:${c.id}`}
                          onClick={() => rescanComic(c.id)}
                        >
                          {busy === `comic:${c.id}` ? "扫描中…" : "重新扫描"}
                        </button>
                        <button className="btn-danger" onClick={() => removeComic(c)}>
                          移除
                        </button>
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="source-footer">
              <button className="btn-primary" onClick={addRoot} disabled={loading}>
                {loading ? "扫描中…" : "+ 添加漫画目录"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
