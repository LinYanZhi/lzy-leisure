import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  onStoryclubPagesDone,
  onStoryclubScanDone,
  type StoryIssue,
} from "../../api";
import { loadStoryPdf, renderFirstPageDataUrl } from "./pdfCover";
import { usePersistScroll } from "../../usePersistScroll";

interface Props {
  /** 当前选中的年份（null = 年份总览；非 null = 该年期数列表） */
  selectedYear: string | null;
  /** 选择/返回年份（App 状态机控制层级） */
  onSelectYear: (year: string | null) => void;
  onOpenIssue: (issue: StoryIssue) => void;
}

function formatSize(bytes: number): string {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} GB`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/**
 * 封面 data URL 模块级缓存（id → dataUrl）：跨组件挂载保留，
 * 切换标签/切出切回不重复批量拉取；批量拉取与后台进度刷新都合并进这里。
 */
const coverCache = new Map<string, string>();

/** 期数封面（url 有值显示，缺失显示书本占位；批量拉取 + 后台补齐即时合并，无逐张 IPC） */
function StoryCoverImage({ url }: { url?: string }) {
  return (
    <div className="story-cover">
      {url && <img src={url} draggable={false} alt="" className="story-cover-img" />}
      {!url && <span className="story-cover-placeholder">书</span>}
    </div>
  );
}

/** 刷新图标（spinning 时旋转动画，表示正在刷新该卡片封面） */
function RefreshIcon({ spinning }: { spinning: boolean }) {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={spinning ? "story-refresh-spin" : undefined}
    >
      <path d="M21 2v6h-6" />
      <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
      <path d="M3 22v-6h6" />
      <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
    </svg>
  );
}

/** 年份卡版本角标：由年份目录名（如「故事会-2019-文摘版」）提取版本类型 */
function yearBadge(year: string): { label: string; cls: string } {
  if (year.includes("文摘版")) return { label: "文摘版", cls: "story-badge-wz" };
  if (year.includes("校园版")) return { label: "校园版", cls: "story-badge-xy" };
  return { label: "正刊", cls: "" };
}

/**
 * 故事会书架：单一路径（根目录下每个子目录 = 一年，内含 PDF 期数）。
 * 两级导航由 App 状态机控制（shelf 年份总览 → year 该年期数 → reader 阅读器）：
 *   selectedYear = null 展示年份卡片网格；非 null 展示该年期数卡片。
 */
export default function StoryShelfView({
  selectedYear,
  onSelectYear,
  onOpenIssue,
}: Props) {
  const [root, setRoot] = useState<string | null>(null);
  const [issues, setIssues] = useState<StoryIssue[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  // 封面 data URL 映射（id → dataUrl）：批量拉取 + 手动刷新即时合并；模块级缓存跨挂载保留
  const [covers, setCovers] = useState<Record<string, string>>(() =>
    Object.fromEntries(coverCache),
  );
  /** 合并封面进缓存并触发渲染（切 tab/重进不重复拉取已缓存的） */
  const commitCovers = useCallback((entries: [string, string][]) => {
    for (const [id, url] of entries) coverCache.set(id, url);
    setCovers(Object.fromEntries(coverCache));
  }, []);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // 两层视图各自独立的滚动位置：年份总览与每个年份详情分别记忆。
  // 原理：两层 DOM 用不同 key（overview / 年份名）→ React 切换层时重建滚动容器，
  // 不会把上一层的 scrollTop 继承过来；usePersistScroll 键不同 → 自动保存旧层位置、
  // 恢复新层位置（依赖键变化的 effect 清理，卸载的旧节点 scrollTop 仍可取到正确值）。
  const bodyRef = useRef<HTMLDivElement | null>(null);
  usePersistScroll(
    selectedYear === null ? "storyclub:overview" : `storyclub:year:${selectedYear}`,
    bodyRef,
    !loading && issues.length > 0,
  );

  const refresh = useCallback(async () => {
    try {
      const [r, list] = await Promise.all([
        api.storyclubGetRoot(),
        api.storyclubListIssues(),
      ]);
      setRoot(r);
      setIssues(list);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // 设置/重扫根路径完成后自动刷新书架
  useEffect(() => {
    const unlisten = onStoryclubScanDone(() => {
      setLoading(false);
      refresh();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refresh]);

  // 后台页数解析完成 → 刷新显示真实页数（前端无需等待，解析完自动补齐）
  useEffect(() => {
    const unlisten = onStoryclubPagesDone(() => refresh());
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refresh]);

  // 年份详情页按 Q 返回年份总览（仅故事会书架层；阅读器有自身的 Q 处理）
  useEffect(() => {
    if (selectedYear === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "q") onSelectYear(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedYear, onSelectYear]);

  const setRootDir = async () => {
    const dir = await api.pickStoryDir();
    if (!dir) return;
    setLoading(true);
    setError("");
    try {
      await api.storyclubSetRoot(dir);
      // 阶段1 已返回（期数已入库），直接刷新显示；不依赖事件兜底
      setLoading(false);
      refresh();
    } catch (e) {
      setError(String(e));
      setLoading(false);
    }
  };

  const rescan = async () => {
    setBusy(true);
    setError("");
    try {
      await api.storyclubRescan();
      // 阶段1 已返回（期数已入库），直接刷新显示
      refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const removeRoot = async () => {
    if (!confirm("确定移除故事会目录及全部期数记录？重新设置目录后可再次扫描导入。")) return;
    setError("");
    try {
      await api.storyclubRemoveRoot();
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const openDir = async (issue: StoryIssue) => {
    try {
      await api.storyclubOpenFolder(issue.id);
    } catch (e) {
      setError(String(e));
    }
  };

  // 单期封面强制刷新中（按钮 loading 用）
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  // 整年封面强制刷新进行中（按钮 loading 用）
  const [refreshingYear, setRefreshingYear] = useState<string | null>(null);
  // 全局封面生成进行中 + 进度（已完成数/总数）
  const [coverGenerating, setCoverGenerating] = useState(false);
  const [coverProgress, setCoverProgress] = useState("");
  // 手机端「⋮ 更多操作」菜单开关
  const [moreOpen, setMoreOpen] = useState(false);

  // 切层（年份总览 ↔ 年份详情）时收起菜单
  useEffect(() => {
    setMoreOpen(false);
  }, [selectedYear]);

  /** 刷新单期封面：前端用 pdf.js 渲染第一页上传覆盖，成功后即时更新该期封面显示 */
  const refreshIssueCover = async (issueId: string) => {
    if (refreshingId) return;
    setRefreshingId(issueId);
    try {
      const doc = await loadStoryPdf(issueId);
      const dataUrl = await renderFirstPageDataUrl(doc);
      await api.storyclubUploadCover(issueId, dataUrl);
      if (mountedRef.current) commitCovers([[issueId, dataUrl]]);
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshingId(null);
    }
  };

  /** 刷新整年封面：前端逐期渲染第一页上传（用户点击触发，非后台任务；每期完成即时更新显示） */
  const refreshYearCovers = async (year: string) => {
    if (refreshingYear) return;
    setRefreshingYear(year);
    setError("");
    try {
      const list =
        year === "未分类"
          ? unclassified ?? []
          : groups.find((g) => g.year === year)?.issues ?? [];
      for (const i of list) {
        const doc = await loadStoryPdf(i.id);
        const dataUrl = await renderFirstPageDataUrl(doc);
        await api.storyclubUploadCover(i.id, dataUrl);
        if (mountedRef.current) commitCovers([[i.id, dataUrl]]);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshingYear(null);
    }
  };

  /** 生成全部缺失封面：对所有年份目录生效（前端逐期渲染上传，只处理尚未生成封面的期数） */
  const genAllCovers = async () => {
    if (coverGenerating) return;
    setCoverGenerating(true);
    setError("");
    try {
      const missing = await api.storyclubMissingCovers();
      const total = missing.length;
      if (total === 0) {
        setCoverProgress("封面已齐全");
        return;
      }
      let done = 0;
      for (const id of missing) {
        try {
          const doc = await loadStoryPdf(id);
          const dataUrl = await renderFirstPageDataUrl(doc);
          await api.storyclubUploadCover(id, dataUrl);
          if (mountedRef.current) commitCovers([[id, dataUrl]]);
        } catch {
          // 单期失败（损坏/无法渲染）跳过，继续下一期
        }
        done++;
        setCoverProgress(`${done}/${total}`);
      }
      setCoverProgress("");
    } catch (e) {
      setError(String(e));
    } finally {
      setCoverGenerating(false);
    }
  };

  /** 年份卡：资源管理器定位并选中该年份目录本身（不进入） */
  const openYearDir = async (year: string) => {
    try {
      await api.storyclubOpenYearFolder(year);
    } catch (e) {
      setError(String(e));
    }
  };

  // 期数已按（年份自然序, 期数序号, 标题）排序，按年份切分保持顺序
  const { groups, unclassified } = useMemo(() => {
    const gs: { year: string; issues: StoryIssue[] }[] = [];
    let un: StoryIssue[] | null = null;
    for (const i of issues) {
      if (!i.year) {
        (un ??= []).push(i);
        continue;
      }
      const last = gs[gs.length - 1];
      if (last && last.year === i.year) last.issues.push(i);
      else gs.push({ year: i.year, issues: [i] });
    }
    if (un) gs.push({ year: "未分类", issues: un });
    return { groups: gs, unclassified: un };
  }, [issues]);

  /** 批量拉取当前视图缺失的封面（只拉未缓存的；命中后即时显示，切年/返回时已拉过的复用） */
  const fetchViewCovers = useCallback(async () => {
    if (groups.length === 0) return;
    const need = new Set<string>();
    if (selectedYear === null) {
      // 年份总览：每组第一期封面
      for (const g of groups) need.add(g.issues[0].id);
    } else {
      const list =
        selectedYear === "未分类"
          ? unclassified ?? []
          : groups.find((g) => g.year === selectedYear)?.issues ?? [];
      for (const i of list) need.add(i.id);
    }
    const missing = [...need].filter((id) => !coverCache.has(id));
    if (missing.length === 0) return;
    try {
      const pairs = await api.storyclubGetCoversBatch(missing);
      if (mountedRef.current) commitCovers(pairs);
    } catch {
      /* 批量拉取失败静默，下次进度事件/重进视图再试 */
    }
  }, [groups, unclassified, selectedYear, commitCovers]);

  // 视图/年份切换时拉取封面
  useEffect(() => {
    fetchViewCovers();
  }, [fetchViewCovers]);

  if (!root && !loading) {
    return (
      <div className="shelf">
        <div className="shelf-empty">
          <p>故事会书架为空</p>
          <p className="muted">点击下方按钮，选择存放《故事会》的根目录（其下每个子目录代表一年）</p>
          <button className="btn-primary story-set-btn" onClick={setRootDir}>
            选择故事会目录
          </button>
        </div>
        {error && <div className="shelf-error">{error}</div>}
      </div>
    );
  }

  return (
    <div className="shelf story-shelf">
      {/* 年份总览层才显示根目录操作栏（二级年份详情参考漫画章节页：只保留简洁头部） */}
      {selectedYear === null && (
        <div className="shelf-toolbar">
          <div className="shelf-roots">
            {root && (
              <div className="root-chip active" title={root}>
                <span className="root-name">故事会</span>
                <span className="root-count">{issues.length} 期</span>
              </div>
            )}
          </div>

          {/* 桌面：完整操作按钮行 */}
          <div className="shelf-toolbar-actions story-actions-desktop">
            <button
              className="btn-ghost"
              onClick={genAllCovers}
              disabled={coverGenerating || busy || refreshingId !== null || refreshingYear !== null || !root}
            >
              {coverGenerating
                ? coverProgress
                  ? `生成封面 ${coverProgress}`
                  : "生成封面中…"
                : "生成封面"}
            </button>
            <button className="btn-ghost" onClick={rescan} disabled={busy || !root}>
              {busy ? "扫描中…" : "重新扫描"}
            </button>
            <button className="btn-ghost" onClick={setRootDir} disabled={loading}>
              {loading ? "扫描中…" : "更换目录"}
            </button>
            <button className="btn-danger" onClick={removeRoot} disabled={!root}>
              移除
            </button>
          </div>

          {/* 手机：⋮ 菜单收纳全部操作，顶栏只占一行 */}
          <div className="story-actions-more">
            <button
              className="vm-icon-btn"
              onClick={() => setMoreOpen((v) => !v)}
              title="更多操作"
              aria-label="更多操作"
            >
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="5" r="1.6" />
                <circle cx="12" cy="12" r="1.6" />
                <circle cx="12" cy="19" r="1.6" />
              </svg>
            </button>
            {moreOpen && (
              <>
                <div className="story-more-backdrop" onClick={() => setMoreOpen(false)} />
                <div className="story-more-menu">
                  <button
                    className="story-more-item"
                    onClick={() => {
                      setMoreOpen(false);
                      void genAllCovers();
                    }}
                    disabled={coverGenerating || busy || refreshingId !== null || refreshingYear !== null || !root}
                  >
                    {coverGenerating
                      ? coverProgress
                        ? `生成封面 ${coverProgress}`
                        : "生成封面中…"
                      : "生成封面"}
                  </button>
                  <button
                    className="story-more-item"
                    onClick={() => {
                      setMoreOpen(false);
                      void rescan();
                    }}
                    disabled={busy || !root}
                  >
                    {busy ? "扫描中…" : "重新扫描"}
                  </button>
                  <button
                    className="story-more-item"
                    onClick={() => {
                      setMoreOpen(false);
                      void setRootDir();
                    }}
                    disabled={loading}
                  >
                    {loading ? "扫描中…" : "更换目录"}
                  </button>
                  <button
                    className="story-more-item story-more-item-danger"
                    onClick={() => {
                      setMoreOpen(false);
                      void removeRoot();
                    }}
                    disabled={!root}
                  >
                    移除
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {error && <div className="shelf-error">{error}</div>}

      {loading ? (
        <div className="center-hint">正在扫描故事会目录…</div>
      ) : issues.length === 0 ? (
        <div className="shelf-empty">
          <p>该目录下没有扫描到期数</p>
          <p className="muted">根目录下每个子目录视为一年，子目录内的 PDF 即为一期</p>
        </div>
      ) : selectedYear === null ? (
        /* ── 第一层：年份总览（每张卡片 = 根目录下的一个年份文件夹） ── */
        <div key="overview" ref={bodyRef} className="story-shelf-body">
          <div className="story-grid">
            {groups.map((g) => (
              <div
                key={g.year}
                className="story-card"
                title={g.year}
                onClick={() => onSelectYear(g.year)}
              >
                <div className="story-cover-wrap">
                  <span className={`story-badge ${yearBadge(g.year).cls}`}>
                    {yearBadge(g.year).label}
                  </span>
                  <StoryCoverImage url={covers[g.issues[0].id]} />
                  <button
                    className="comic-open-btn"
                    title="打开所在目录"
                    onClick={(e) => {
                      e.stopPropagation();
                      openYearDir(g.year);
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                    </svg>
                  </button>
                  <button
                    className="story-refresh-btn"
                    title="重新生成该年全部封面"
                    disabled={refreshingYear !== null || coverGenerating}
                    onClick={(e) => {
                      e.stopPropagation();
                      refreshYearCovers(g.year);
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                  >
                    <RefreshIcon spinning={refreshingYear === g.year} />
                  </button>
                </div>
                <div className="story-title" title={g.year}>
                  {g.year}
                </div>
                <div className="story-meta">{g.issues.length} 期</div>
              </div>
            ))}
          </div>
        </div>
      ) : (
        /* ── 第二层：某年详情（该年全部期数 PDF 卡片） ── */
        <div key={selectedYear} ref={bodyRef} className="story-shelf-body">
          <div className="story-year-header">
            <button className="btn-ghost" onClick={() => onSelectYear(null)}>
              ← 全部年份
            </button>
            <h2 className="story-year-title">
              {selectedYear}
              <span className="story-year-count">
                {(selectedYear === "未分类"
                  ? unclassified
                  : groups.find((g) => g.year === selectedYear)?.issues
                )?.length ?? 0}{" "}
                期
              </span>
            </h2>
          </div>
          <div className="story-grid">
            {(selectedYear === "未分类"
              ? unclassified ?? []
              : groups.find((g) => g.year === selectedYear)?.issues ?? []
            ).map((i) => (
              <div
                key={i.id}
                className="story-card"
                title={i.title}
                onClick={() => onOpenIssue(i)}
              >
                <div className="story-cover-wrap">
                  <StoryCoverImage url={covers[i.id]} />
                  <button
                    className="comic-open-btn"
                    title="打开所在目录"
                    onClick={(e) => {
                      e.stopPropagation();
                      openDir(i);
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                    </svg>
                  </button>
                  <button
                    className="story-refresh-btn"
                    title="重新生成封面"
                    disabled={refreshingId !== null || refreshingYear !== null || coverGenerating}
                    onClick={(e) => {
                      e.stopPropagation();
                      refreshIssueCover(i.id);
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                  >
                    <RefreshIcon spinning={refreshingId === i.id} />
                  </button>
                  {i.reading_progress > 0 && (
                    <div className="comic-progress">
                      读到 {i.reading_progress}
                      {i.page_count > 0 ? ` / ${i.page_count}` : ""} 页
                    </div>
                  )}
                </div>
                <div className="story-title" title={i.title}>
                  {i.title}
                </div>
                <div className="story-meta">
                  {i.page_count > 0 ? `${i.page_count} 页` : "页数解析中"} · {formatSize(i.size)}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
