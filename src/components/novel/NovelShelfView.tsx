import { useCallback, useEffect, useRef, useState } from "react";
import { useData } from "@glbt/appkit-ui";
import {
  api,
  onNovelScanDone,
  type Novel,
  type NovelRootDir,
} from "../../api";
import { store } from "../../data";

interface Props {
  onOpenNovel: (novel: Novel) => void;
}

/** 小说封面（进入视口才加载；后端解析 EPUB 提取封面缓存） */
function NovelCover({ novelId, className }: { novelId: string; className: string }) {
  const elRef = useRef<HTMLElement | null>(null);
  const setEl = useCallback((el: HTMLElement | null) => {
    elRef.current = el;
  }, []);
  const [src, setSrc] = useState<string>();

  useEffect(() => {
    let cancelled = false;
    const el = elRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            io.disconnect();
            api
              .getNovelCoverDataUrl(novelId)
              .then((url) => {
                if (!cancelled) setSrc(url);
              })
              .catch(() => {});
            break;
          }
        }
      },
      { rootMargin: "400px 0px" },
    );
    io.observe(el);
    return () => {
      cancelled = true;
      io.disconnect();
    };
  }, [novelId]);

  if (!src) {
    return <div ref={setEl} className={className} style={{ background: "var(--bg-badge)" }} />;
  }
  return <img ref={setEl} className={className} src={src} draggable={false} alt="" />;
}

/** 续读徽标文案（chapter 为章节索引字符串） */
function resumeBadge(n: Novel): string {
  if (n.chapter === "") return "";
  const idx = Number(n.chapter);
  const ch = n.chapters[idx];
  const name = ch?.title || `第 ${idx + 1} 章`;
  if (n.reading_pos > 0) return `续读 ${name}`;
  return name;
}

/** 小说书架：导入源管理 + 封面墙 + 单文件导入 */
export default function NovelShelfView({ onOpenNovel }: Props) {
  const [rootDirs, setRootDirs] = useState<NovelRootDir[]>([]);
  const [novels, setNovels] = useState<Novel[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [activeRoot, setActiveRoot] = useData(store.keys["novel-active-root"]);
  const [showManager, setShowManager] = useState(false);
  const [managerError, setManagerError] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [dirs, list] = await Promise.all([api.getNovelRoots(), api.listNovels()]);
      setRootDirs(dirs);
      setNovels(list);
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

  // 目录扫描完成（后端抛后台，事件回来刷新）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onNovelScanDone(() => {
      setLoading(false);
      refresh();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [refresh]);

  // ── 管理弹窗 ──

  const openManager = () => {
    setManagerError("");
    setShowManager(true);
  };

  const addRoot = async () => {
    const dir = await api.pickNovelDir();
    if (!dir) return;
    setLoading(true);
    setError("");
    setManagerError("");
    try {
      await api.addNovelRoot(dir);
    } catch (e) {
      setError(String(e));
      setManagerError(String(e));
      setLoading(false);
    }
  };

  const addFile = async () => {
    const file = await api.pickNovelFile();
    if (!file) return;
    setLoading(true);
    setError("");
    setManagerError("");
    try {
      await api.addNovel(file);
      await refresh();
    } catch (e) {
      setError(String(e));
      setManagerError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const rescanRoot = async (path: string) => {
    const key = `root:${path}`;
    setBusy(key);
    setManagerError("");
    try {
      await api.rescanNovelRoot(path);
    } catch (e) {
      setManagerError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const removeRoot = async (path: string) => {
    if (!confirm(`确定移除小说源「${path}」及其全部书籍记录？不删除本地文件，重新添加后可再扫描。`)) return;
    setManagerError("");
    try {
      await api.removeNovelRoot(path);
      await refresh();
    } catch (e) {
      setManagerError(String(e));
    }
  };

  const removeNovel = async (n: Novel) => {
    if (!confirm(`确定从书架移除「${n.title}」？不删除本地文件。`)) return;
    setManagerError("");
    try {
      await api.deleteNovel(n.id);
      await refresh();
    } catch (e) {
      setManagerError(String(e));
    }
  };

  const renameNovel = async (n: Novel) => {
    const title = prompt("输入新书名：", n.title);
    if (!title || title.trim() === "" || title.trim() === n.title) return;
    try {
      await api.renameNovel(n.id, title.trim());
      await refresh();
    } catch (e) {
      setManagerError(String(e));
    }
  };

  const openDir = async (n: Novel) => {
    try {
      await api.openNovelFolder(n.id);
    } catch (e) {
      setError(String(e));
    }
  };

  const visibleNovels = activeRoot
    ? novels.filter((n) => n.root_dir === activeRoot)
    : novels;

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
              <span className="root-count">{d.novel_count}</span>
            </div>
          ))}
        </div>
        <div className="shelf-toolbar-actions">
          <button className="btn-ghost" onClick={addFile} title="导入单个 EPUB / TXT 文件">
            + 导入书籍
          </button>
          <button className="btn-ghost" onClick={openManager} title="管理导入的小说文件夹">
            小说源管理
          </button>
        </div>
      </div>

      {error && <div className="shelf-error">{error}</div>}

      {novels.length === 0 && !loading ? (
        <div className="shelf-empty">
          <p>书架为空</p>
          <p className="muted">
            点击右上角「小说源管理」添加存放 EPUB / TXT 的文件夹，或「+ 导入书籍」导入单个文件
          </p>
        </div>
      ) : (
        <div className="shelf-grid">
          {visibleNovels.map((n) => {
            const badge = resumeBadge(n);
            return (
              <div
                key={n.id}
                className="comic-card"
                title="点击阅读"
                onClick={() => onOpenNovel(n)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  openDir(n);
                }}
              >
                <div className="comic-cover-wrap">
                  <NovelCover novelId={n.id} className="novel-cover" />
                  <button
                    className="comic-open-btn"
                    title="打开所在目录"
                    onClick={(e) => {
                      e.stopPropagation();
                      openDir(n);
                    }}
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                    </svg>
                  </button>
                  {badge && <div className="comic-progress">{badge}</div>}
                </div>
                <div className="comic-title" title={n.title}>
                  {n.title}
                </div>
                <div className="comic-meta">
                  {n.author || "佚名"} · {n.chapter_count} 章
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
              <span className="source-title">小说源管理</span>
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

              <div className="source-section-title">已导入的书籍</div>
              {novels.length === 0 ? (
                <div className="source-empty">暂无书籍</div>
              ) : (
                <div className="source-list">
                  {novels.map((n) => (
                    <div className="source-row" key={n.id}>
                      <span className="source-path" title={n.path}>
                        {n.title}
                        {n.author ? `（${n.author}）` : ""}
                        <span className="muted"> · {n.chapter_count} 章</span>
                      </span>
                      <span className="source-actions">
                        <button className="btn-ghost" onClick={() => renameNovel(n)}>
                          改名
                        </button>
                        <button className="btn-danger" onClick={() => removeNovel(n)}>
                          移除
                        </button>
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="source-footer">
              <button className="btn-ghost" onClick={addFile} disabled={loading}>
                {loading ? "导入中…" : "+ 导入单个 EPUB / TXT"}
              </button>
              <button className="btn-primary" onClick={addRoot} disabled={loading}>
                {loading ? "扫描中…" : "+ 添加小说目录"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
