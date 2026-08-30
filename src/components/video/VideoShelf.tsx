import { useCallback, useEffect, useState } from "react";
import {
  api,
  onVideoScanDone,
  type Actor,
  type Tag,
  type TagGroup,
  type Video,
  type VideoQuery,
  type VideoRootDir,
  type VideoSeries,
} from "../../api";
import CoverImage from "./CoverImage";
import { KIND_LABEL } from "./kinds";
import { ensureVideoCover, probeVideoMeta } from "./videoMedia";
import type { VideoPage } from "./VideoSidebar";

interface Props {
  page: VideoPage;
  /** 左侧导航触发的操作（源管理弹窗 / 添加视频），由 App 转发 */
  action: "manager" | "add" | null;
  onActionHandled: () => void;
  onOpenVideo: (video: Video) => void;
  onPlayVideo: (video: Video) => void;
  onOpenSeries: (series: VideoSeries) => void;
}

/** 秒数格式化为 mm:ss / hh:mm:ss */
function fmtSec(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = m < 10 ? `0${m}` : `${m}`;
  const ss = r < 10 ? `0${r}` : `${r}`;
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

/** duration（HH:MM:SS 或 MM:SS）→ 秒 */
function durToSec(dur: string): number {
  const parts = dur.split(":").map((x) => parseFloat(x) || 0);
  let sec = 0;
  for (const p of parts) sec = sec * 60 + p;
  return sec;
}

/** 分辨率友好显示：4K / 2K / 1080P / 720P，其余退回 宽×高 */
function resLabel(w?: number | null, h?: number | null): string {
  if (!w || !h) return "";
  if (w >= 3840) return "4K";
  if (w >= 2560) return "2K";
  if (w >= 1920) return "1080P";
  if (w >= 1280) return "720P";
  return `${w}×${h}`;
}

/** 视频卡片悬停操作 */
function iconBtn(title: string, onClick: (e: React.MouseEvent) => void, icon: React.ReactNode) {
  return (
    <button
      className="vs-card-btn"
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        onClick(e);
      }}
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        {icon}
      </svg>
    </button>
  );
}

const ICONS = {
  play: <polygon points="5 3 19 12 5 21 5 3" />,
  folder: <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />,
  refresh: (
    <>
      <path d="M21 2v6h-6" />
      <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
      <path d="M3 22v-6h6" />
      <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
    </>
  ),
  trash: (
    <>
      <path d="M3 6h18" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </>
  ),
};

/** 当前页面的显示名（主区标题） */
export function pageTitle(page: VideoPage, roots: VideoRootDir[]): string {
  switch (page.name) {
    case "all":
      return "全部视频";
    case "series":
      return "剧集";
    case "kind":
      return KIND_LABEL[page.kind] ?? "未知种类";
    case "root":
      return roots.find((r) => r.path === page.path)?.name ?? "导入路径";
    default:
      return "";
  }
}

/**
 * 视频主容器：左侧导航（VideoSidebar 由 App 渲染）驱动 page，
 * 这里负责数据加载、搜索/筛选、网格与系列墙、源管理弹窗。
 */
export default function VideoShelf({ page, action, onActionHandled, onOpenVideo, onPlayVideo, onOpenSeries }: Props) {
  const [videos, setVideos] = useState<Video[]>([]);
  const [actors, setActors] = useState<Actor[]>([]);
  const [tags, setTags] = useState<Tag[]>([]);
  const [groups, setGroups] = useState<TagGroup[]>([]);
  const [seriesList, setSeriesList] = useState<VideoSeries[]>([]);
  const [roots, setRoots] = useState<VideoRootDir[]>([]);

  const [search, setSearch] = useState("");
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [selectedActors, setSelectedActors] = useState<string[]>([]);
  const [matchAll, setMatchAll] = useState(false);
  // 筛选面板展开状态（演员/标签 chips，默认收起保持页面干净）
  const [showFilters, setShowFilters] = useState(false);

  const [videoLoading, setVideoLoading] = useState(false);
  const [error, setError] = useState("");
  // 封面版本：重新生成封面后 +1 刷新对应卡片
  const [coverRev, setCoverRev] = useState(0);

  // 视频源管理弹窗
  const [showManager, setShowManager] = useState(false);
  const [managerRoots, setManagerRoots] = useState<VideoRootDir[]>([]);
  const [managerLoading, setManagerLoading] = useState(false);
  const [managerError, setManagerError] = useState("");
  const [busy, setBusy] = useState("");

  // 切导航时重置搜索与筛选
  useEffect(() => {
    setSearch("");
    setSelectedTags([]);
    setSelectedActors([]);
    setMatchAll(false);
    setShowFilters(false);
  }, [page]);

  const loadVideos = useCallback(async () => {
    setVideoLoading(true);
    try {
      const q: VideoQuery = {
        title: search || undefined,
        tag_ids: selectedTags,
        match_all: matchAll,
        actor_ids: selectedActors,
        kinds: page.name === "kind" ? [page.kind] : undefined,
        root_dir: page.name === "root" ? page.path : undefined,
      };
      setVideos(await api.listVideos(q));
      setError("");
    } catch (e) {
      setError(String(e));
    } finally {
      setVideoLoading(false);
    }
  }, [search, selectedTags, matchAll, selectedActors, page]);

  const loadAll = useCallback(async () => {
    try {
      const [a, t, g, r, s] = await Promise.all([
        api.listActors(),
        api.listTags(),
        api.listTagGroups(),
        api.getVideoRoots(),
        api.listSeries(),
      ]);
      setActors(a);
      setTags(t);
      setGroups(g);
      setRoots(r);
      setSeriesList(s);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    loadAll();
  }, [loadAll]);

  useEffect(() => {
    const t = setTimeout(loadVideos, 200);
    return () => clearTimeout(t);
  }, [loadVideos]);

  // 元数据补全：新扫描视频缺时长/分辨率，前端懒加载读取并写回后端（仅补缺项）
  useEffect(() => {
    if (videoLoading || videos.length === 0) return;
    const missing = videos.filter((v) => !v.duration);
    if (missing.length === 0) return;
    let cancelled = false;
    for (const v of missing) {
      probeVideoMeta(v)
        .then((patch) => {
          if (cancelled || !patch) return;
          setVideos((prev) =>
            prev.map((x) =>
              x.id === v.id
                ? {
                    ...x,
                    duration: patch.duration || x.duration,
                    frame_width: patch.frameWidth ?? x.frame_width,
                    frame_height: patch.frameHeight ?? x.frame_height,
                  }
                : x,
            ),
          );
        })
        .catch(() => {});
    }
    return () => {
      cancelled = true;
    };
  }, [videos, videoLoading]);

  // 目录扫描完成（添加/重新扫描）后自动刷新：源列表 + 当前网格
  useEffect(() => {
    let un: (() => void) | undefined;
    onVideoScanDone(() => {
      void loadAll();
      void loadVideos();
    }).then((fn) => (un = fn));
    return () => un?.();
  }, [loadAll, loadVideos]);

  // 左侧导航触发的操作：打开源管理弹窗 / 添加单个视频
  useEffect(() => {
    if (action === "manager") {
      setShowManager(true);
      onActionHandled();
    } else if (action === "add") {
      void addVideo();
      onActionHandled();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [action]);

  // ── 源管理弹窗 ──

  const loadManagerRoots = useCallback(async () => {
    setManagerLoading(true);
    setManagerError("");
    try {
      setManagerRoots(await api.getVideoRoots());
    } catch (e) {
      setManagerError(String(e));
    } finally {
      setManagerLoading(false);
    }
  }, []);

  useEffect(() => {
    if (showManager) loadManagerRoots();
  }, [showManager, loadManagerRoots]);

  const createSeriesFromRoot = async (path: string) => {
    setBusy(`series:${path}`);
    setManagerError("");
    try {
      await api.createSeriesFromDir(path);
      setShowManager(false);
      await Promise.all([loadAll(), loadManagerRoots()]);
    } catch (e) {
      setManagerError(String(e));
    } finally {
      setBusy("");
    }
  };

  // 添加视频目录：后端登记后立即返回，扫描在后台执行，完成后 video-scan-done 事件自动刷新
  const addRoot = async () => {
    const dir = await api.pickVideoDir();
    if (!dir) return;
    setManagerError("");
    setError("");
    try {
      await api.addVideoRoot(dir);
      setShowManager(false);
      await loadAll();
    } catch (e) {
      setManagerError(String(e));
      await loadManagerRoots();
    }
  };

  const rescanRoot = async (path: string) => {
    setBusy(`root:${path}`);
    setManagerError("");
    try {
      await api.rescanVideoRoot(path);
      await Promise.all([loadAll(), loadManagerRoots()]);
    } catch (e) {
      setManagerError(String(e));
    } finally {
      setBusy("");
    }
  };

  const removeRoot = async (path: string) => {
    if (!confirm(`确定移除视频目录「${path}」？\n其下所有视频记录将从书库移除（不会删除本地文件）。`)) return;
    setManagerError("");
    try {
      await api.removeVideoRoot(path);
      await Promise.all([loadAll(), loadManagerRoots()]);
    } catch (e) {
      setManagerError(String(e));
    }
  };

  const addVideo = async () => {
    const path = await api.pickVideoFile();
    if (!path) return;
    setError("");
    try {
      await api.addVideo(path);
      await Promise.all([loadAll(), loadVideos()]);
    } catch (e) {
      setError(String(e));
    }
  };

  // ── 交互 ──

  const toggleTag = (id: string) => {
    setSelectedTags((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  };
  const toggleActor = (id: string) => {
    setSelectedActors((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  };
  const clearFilters = () => {
    setSearch("");
    setSelectedTags([]);
    setSelectedActors([]);
    setMatchAll(false);
  };

  const openFolder = async (v: Video) => {
    try {
      await api.openVideoFolder(v.id);
    } catch (e) {
      setError(String(e));
    }
  };

  const regenerate = async (v: Video) => {
    try {
      const ok = await ensureVideoCover(v, true);
      if (ok) setCoverRev((n) => n + 1);
      else setError("无法生成封面（浏览器不支持该视频格式）");
    } catch (e) {
      setError(String(e));
    }
  };

  const removeVideo = async (v: Video) => {
    if (!confirm(`确定删除「${v.title}」的记录？\n（不会删除本地文件）`)) return;
    try {
      await api.deleteVideo(v.id);
      setVideos((prev) => prev.filter((x) => x.id !== v.id));
    } catch (e) {
      setError(String(e));
    }
  };

  const removeSeries = async (s: VideoSeries) => {
    if (!confirm(`确定删除剧集「${s.title}」？\n其下 ${s.video_count} 个视频将解除归属（不会删除本地文件）。`)) return;
    try {
      await api.deleteSeries(s.id);
      setSeriesList((prev) => prev.filter((x) => x.id !== s.id));
    } catch (e) {
      setError(String(e));
    }
  };

  const hasFilters = search !== "" || selectedTags.length > 0 || selectedActors.length > 0;
  const title = pageTitle(page, roots);
  const showSeries = page.name === "series";

  return (
    <div className="vs-page">
      {/* ── 主区工具条：页面名 + 搜索 + 筛选 + 管理 ── */}
      <div className="vs-toolbar">
        <div className="vs-toolbar-left">
          <span className="vs-page-title">{title}</span>
          {!showSeries && hasFilters && (
            <button className="btn btn-sm" onClick={clearFilters}>清空筛选</button>
          )}
        </div>
        <div className="vs-toolbar-right">
          {!showSeries && (
            <input
              className="vs-search"
              placeholder="搜索标题…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          )}
          {!showSeries && (actors.length > 0 || tags.length > 0) && (
            <button
              className={`btn-ghost v-filter-btn${showFilters ? " active" : ""}${hasFilters ? " has" : ""}`}
              onClick={() => setShowFilters((s) => !s)}
              title="按演员 / 标签筛选"
            >
              筛选
            </button>
          )}
          <button className="btn-ghost vs-manager-btn" onClick={() => setShowManager(true)} title="管理导入的视频目录">
            视频源管理
          </button>
        </div>
      </div>

      {/* ── 筛选区（收起默认隐藏；有选中项时保持展开） ── */}
      {!showSeries && (showFilters || hasFilters) && (actors.length > 0 || tags.length > 0) && (
        <div className="vs-filters">
          {actors.length > 0 && (
            <div className="vs-filter-row">
              <span className="vs-filter-label">演员</span>
              <div className="vs-filter-chips">
                {actors.map((a) => (
                  <button
                    key={a.id}
                    className={`vs-chip${selectedActors.includes(a.id) ? " active" : ""}`}
                    onClick={() => toggleActor(a.id)}
                  >
                    {a.name}
                  </button>
                ))}
              </div>
            </div>
          )}
          {tags.length > 0 &&
            groups.map((g) => {
              const gtags = tags.filter((t) => t.group_id === g.id);
              if (gtags.length === 0) return null;
              return (
                <div className="vs-filter-row" key={g.id}>
                  <span className="vs-filter-label">{g.name}</span>
                  <div className="vs-filter-chips">
                    {gtags.map((t) => (
                      <button
                        key={t.id}
                        className={`vs-chip${selectedTags.includes(t.id) ? " active" : ""}`}
                        style={selectedTags.includes(t.id) ? { borderColor: t.color, color: t.color } : undefined}
                        onClick={() => toggleTag(t.id)}
                      >
                        {t.name}
                      </button>
                    ))}
                  </div>
                </div>
              );
            })}
          {hasFilters && (
            <div className="vs-filter-actions">
              <label className="vs-check">
                <input type="checkbox" checked={matchAll} onChange={(e) => setMatchAll(e.target.checked)} />
                标签需全部匹配
              </label>
            </div>
          )}
        </div>
      )}

      {error && <div className="vs-error">{error}</div>}

      {/* ── 系列墙 ── */}
      {showSeries ? (
        seriesList.length === 0 ? (
          <div className="vs-empty">
            <p>暂无剧集</p>
            <p className="muted">点击左下角「视频源管理」，在某个视频目录上点「创建剧集」即可一键归组</p>
          </div>
        ) : (
          <div className="vs-series-grid">
            {seriesList.map((s) => (
              <div key={s.id} className="vs-series-card" onClick={() => onOpenSeries(s)} title={s.title}>
                <div className="vs-cover-wrap">
                  {s.cover_video_id ? (
                    <CoverImage videoId={s.cover_video_id} className="vs-cover" />
                  ) : (
                    <div className="vs-series-no-cover">无封面</div>
                  )}
                  <span className="vs-series-count">{s.video_count} 集</span>
                  <div className="vs-card-hover">
                    {iconBtn("打开剧集", () => onOpenSeries(s), ICONS.play)}
                    {iconBtn("删除剧集", () => void removeSeries(s), ICONS.trash)}
                  </div>
                </div>
                <div className="vs-title" title={s.title}>{s.title}</div>
              </div>
            ))}
          </div>
        )
      ) : videoLoading && videos.length === 0 ? (
        <div className="vs-empty">加载中…</div>
      ) : videos.length === 0 ? (
        <div className="vs-empty">
          <p>{hasFilters ? "没有符合条件的视频" : "该分类下暂无视频"}</p>
          <p className="muted">
            {hasFilters ? "调整筛选条件，或清空筛选" : "点击左下角「视频源管理」添加本地视频目录"}
          </p>
        </div>
      ) : (
        <div className="vs-grid">
          {videos.map((v) => {
            // 时长放在标题下方小字（meta）里，封面不再叠加角标
            const meta = [v.duration, resLabel(v.frame_width, v.frame_height), v.year]
              .filter(Boolean)
              .join(" · ") || v.file_type;
            return (
              <div key={v.id} className="vs-card" onClick={() => onOpenVideo(v)} title={v.title}>
                <div className="vs-cover-wrap">
                  <CoverImage videoId={v.id} version={coverRev} className="vs-cover" />
                  {v.subtitle_path && (
                    <span className="vs-sub-badge" title="有同名字幕文件">字幕</span>
                  )}
                  {v.progress > 0 && v.duration && (
                    <div
                      className="vs-progress-bar"
                      title={`已播放 ${fmtSec(v.progress)} / ${v.duration}`}
                    >
                      <div
                        className="vs-progress-fill"
                        style={{ width: `${Math.min(100, (v.progress / durToSec(v.duration)) * 100)}%` }}
                      />
                    </div>
                  )}
                  <div className="vs-card-hover">
                    {iconBtn("播放", () => onPlayVideo(v), ICONS.play)}
                    {iconBtn("打开所在目录", () => void openFolder(v), ICONS.folder)}
                    {iconBtn("重新生成封面", () => void regenerate(v), ICONS.refresh)}
                    {iconBtn("删除记录", () => void removeVideo(v), ICONS.trash)}
                  </div>
                </div>
                <div className="vs-title" title={v.title}>{v.title}</div>
                {v.episode && <div className="vs-episode" title="集数">{v.episode}</div>}
                <div className="vs-meta">{meta}</div>
              </div>
            );
          })}
        </div>
      )}

      {/* ── 视频源管理弹窗 ── */}
      {showManager && (
        <div
          className="source-overlay"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowManager(false);
          }}
        >
          <div className="source-modal">
            <div className="source-header">
              <span className="source-title">视频源管理</span>
              <button className="source-close" onClick={() => setShowManager(false)} title="关闭">
                ✕
              </button>
            </div>
            <div className="source-body">
              {managerError && <div className="shelf-error">{managerError}</div>}

              <div className="source-section-title">导入的路径</div>
              {managerLoading ? (
                <div className="source-empty">加载中…</div>
              ) : managerRoots.length === 0 ? (
                <div className="source-empty">暂无导入路径</div>
              ) : (
                <div className="source-list">
                  {managerRoots.map((d) => (
                    <div className="source-row" key={d.path}>
                      <span className="source-path" title={d.path}>
                        {d.path}
                      </span>
                      <span className="source-actions">
                        <button
                          className="btn-ghost"
                          disabled={busy === `series:${d.path}`}
                          onClick={() => createSeriesFromRoot(d.path)}
                          title="将该目录（含子目录）下所有视频归为一个剧集，目录名作标题"
                        >
                          {busy === `series:${d.path}` ? "创建中…" : "创建剧集"}
                        </button>
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

              <div className="source-section-title">说明</div>
              <div className="source-empty">
                视频目录下的子目录会一并扫描；重新扫描会同步移除磁盘上已不存在的视频记录。移除路径会连同其下视频记录一并从书库移除（不删除本地文件）。
              </div>
            </div>
            <div className="source-footer">
              <button className="btn-primary" onClick={addRoot}>
                + 添加视频目录
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
