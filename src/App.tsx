import { useState, useCallback, useEffect } from "react";
import {
  setWindowPin,
  AppLayout,
  ShortcutsPanel,
  AboutPanel,
  DataManagerPanel,
  TabBar,
  useData,
} from "@glbt/appkit-ui";
import type { SettingsTab, TabItem } from "@glbt/appkit-ui";
import { store } from "./data";
import { isWeb, api } from "./api";
import type { Comic, Novel, StoryIssue, Video, VideoRootDir, VideoSeries } from "./api";
import { useIsMobile } from "./useIsMobile";
import ComicShelfView from "./components/comic/ShelfView";
import ComicChapterView from "./components/comic/ChapterView";
import ComicReaderView from "./components/comic/ReaderView";
import NovelShelfView from "./components/novel/NovelShelfView";
import NovelReaderView from "./components/novel/NovelReaderView";
import StoryShelfView from "./components/storyclub/StoryShelfView";
import StoryReader from "./components/storyclub/StoryReader";
import VideoShelf from "./components/video/VideoShelf";
import VideoSidebar, { type VideoPage } from "./components/video/VideoSidebar";
import MobileVideoLayout from "./components/video/MobileVideoLayout";
import VideoDetailView from "./components/video/VideoDetailView";
import VideoSeriesView from "./components/video/VideoSeriesView";
import PlayerView from "./components/video/PlayerView";
import ActorsView from "./components/video/ActorsView";
import TagsView from "./components/video/TagsView";
import "@glbt/appkit-ui/styles";
import "./App.css";

// ── 顶层模块标签（漫画 / 视频 / 小说 / 故事会） ──
const MODULE_TABS: TabItem[] = [
  { id: "comics", label: "漫画" },
  { id: "videos", label: "视频" },
  { id: "novels", label: "小说" },
  { id: "storyclub", label: "故事会" },
];

const MODULE_LABELS: Record<string, string> = {
  comics: "漫画",
  videos: "视频",
  novels: "小说",
  storyclub: "故事会",
};

/** 底部导航图标（移动端模块切换） */
const MODULE_ICONS: Record<string, React.ReactNode> = {
  comics: (
    <svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  ),
  videos: (
    <svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="5" width="14" height="14" rx="2" />
      <path d="M22 8.5v7l-6-3.5z" />
    </svg>
  ),
  novels: (
    <svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z" />
      <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z" />
    </svg>
  ),
  storyclub: (
    <svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 18v-6a9 9 0 0 1 18 0v6" />
      <path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3zM3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z" />
    </svg>
  ),
};

/** 移动端底部模块导航（PC 隐藏） */
function MobileModuleNav({ active, onChange }: { active: string; onChange: (id: string) => void }) {
  return (
    <nav className="app-mobile-nav" aria-label="模块导航">
      {MODULE_TABS.map((t) => (
        <button
          key={t.id}
          className={`amn-btn${active === t.id ? " active" : ""}`}
          onClick={() => onChange(t.id)}
        >
          {MODULE_ICONS[t.id]}
          <span>{t.label}</span>
        </button>
      ))}
    </nav>
  );
}

type ComicView =
  | { name: "shelf" }
  | { name: "series"; series: Comic }
  | { name: "reader"; comic: Comic; fromSeries?: Comic };

type NovelView = { name: "shelf" } | { name: "reader"; novel: Novel };

type StoryView =
  | { name: "shelf" }
  | { name: "year"; year: string }
  | { name: "reader"; issue: StoryIssue; year: string };

function App() {
  const [activeModule, setActiveModule] = useData(store.keys["module-tab"]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // 手机端（视口 ≤768px）视频模块切换为「顶栏 + 底部导航 + 抽屉」壳布局
  const isMobile = useIsMobile();

  // ── 一次性迁移：早期未注册的 localStorage 旧 key（下划线）→ 注册后的连字符 key ──
  useEffect(() => {
    const MIGRATIONS: [string, string][] = [
      ["leisure_web_token", "leisure-web-token"],
      ["leisure_comic_active_root", "leisure-comic-active-root"],
      ["leisure_novel_active_root", "leisure-novel-active-root"],
      ["leisure_novel_font_size", "leisure-novel-font-size"],
    ];
    try {
      for (const [oldKey, newKey] of MIGRATIONS) {
        const v = localStorage.getItem(oldKey);
        if (v === null) continue;
        if (localStorage.getItem(newKey) === null) localStorage.setItem(newKey, v);
        localStorage.removeItem(oldKey);
      }
    } catch {}
  }, []);

  // ── 局域网访问链接（仅桌面窗口展示；浏览器模式无此页签） ──
  const [webUrls, setWebUrls] = useState<string[]>([]);
  const [webPassword, setWebPassword] = useState("");
  useEffect(() => {
    if (isWeb) return;
    api
      .getWebUrls()
      .then((r) => {
        setWebUrls(r.urls);
        setWebPassword(r.password);
      })
      .catch(() => {});
  }, []);

  // ── 漫画模块视图状态 ──
  const [comicView, setComicView] = useState<ComicView>({ name: "shelf" });
  // ── 小说模块视图状态 ──
  const [novelView, setNovelView] = useState<NovelView>({ name: "shelf" });
  // ── 故事会模块视图状态 ──
  const [storyView, setStoryView] = useState<StoryView>({ name: "shelf" });
  // ── 视频模块：左侧导航页 / 详情页 / 播放器 / 剧集详情 ──
  const [videoPage, setVideoPage] = useState<VideoPage>({ name: "all" });
  const [videoDetail, setVideoDetail] = useState<Video | null>(null);
  const [playing, setPlaying] = useState<Video | null>(null);
  // 剧集连播：当前视频所属剧集的有序列表 + 当前下标
  const [playlist, setPlaylist] = useState<Video[]>([]);
  const [playIndex, setPlayIndex] = useState(0);
  const [videoSeriesView, setVideoSeriesView] = useState<VideoSeries | null>(null);
  // 左侧导航触发的操作（转发给 VideoShelf）
  const [videoAction, setVideoAction] = useState<"manager" | "add" | null>(null);
  // 左侧导航用数据（导入路径）
  const [videoRoots, setVideoRoots] = useState<VideoRootDir[]>([]);
  // 数据变更后强制刷新影片网格
  const [shelfKey, setShelfKey] = useState(0);
  const notifyChanged = useCallback(() => setShelfKey((n) => n + 1), []);

  // 播放视频：若属于某剧集，拉取剧集有序列表支持连播
  const playVideo = useCallback(async (video: Video) => {
    setPlaying(video);
    setPlaylist([]);
    setPlayIndex(0);
    if (!video.series_id) return;
    try {
      const d = await api.getSeries(video.series_id);
      const idx = d.videos.findIndex((x) => x.id === video.id);
      if (idx >= 0) {
        setPlaylist(d.videos);
        setPlayIndex(idx);
      }
    } catch {}
  }, []);

  // 连播切集（上一集 / 下一集 / 播完自动下一集）
  const switchVideo = useCallback(
    (video: Video) => {
      setPlaying(video);
      const idx = playlist.findIndex((x) => x.id === video.id);
      setPlayIndex(idx >= 0 ? idx : 0);
    },
    [playlist],
  );

  // 侧边栏依赖的数据（源列表）；shelfKey 变化或首次挂载时刷新
  const loadVideoMeta = useCallback(async () => {
    try {
      setVideoRoots(await api.getVideoRoots());
    } catch {}
  }, []);
  useEffect(() => {
    loadVideoMeta();
  }, [loadVideoMeta, shelfKey]);

  // 顶栏左侧：PC 显示模块 tab 栏；手机端模块切换走底部导航，这里只显示当前模块名
  const moduleBar = isMobile ? (
    <div className="topbar-module-name">{MODULE_LABELS[activeModule] ?? ""}</div>
  ) : (
    <TabBar
      tabs={MODULE_TABS}
      activeTab={activeModule}
      onTabChange={setActiveModule}
      disabled={settingsOpen}
    />
  );

  // 顶栏滚轮：切换顶层模块
  const handleTopBarWheel = useCallback(
    (e: React.WheelEvent) => {
      if (settingsOpen) return;
      const idx = MODULE_TABS.findIndex((t) => t.id === activeModule);
      if (e.deltaY > 0) {
        if (idx < MODULE_TABS.length - 1) setActiveModule(MODULE_TABS[idx + 1].id);
      } else {
        if (idx > 0) setActiveModule(MODULE_TABS[idx - 1].id);
      }
    },
    [activeModule, settingsOpen],
  );

  // 点击局域网链接 → 系统默认浏览器打开
  const openWeb = (url: string) => {
    api.openWebUrl(url).catch(() => {});
  };

  const tabs: SettingsTab[] = [
    {
      id: "data-management",
      label: "数据管理",
      icon: <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3" /><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" /><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" /></svg>,
      content: <DataManagerPanel appName="休闲时光" />,
    },
    {
      id: "shortcuts",
      label: "快捷键",
      icon: <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect x="2" y="4" width="20" height="16" rx="2" /><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M8 12h.01M12 12h.01M16 12h.01M6 16h.01M10 16h.01M12 16h.01" /></svg>,
      content: <ShortcutsPanel />,
    },
    {
      id: "about",
      label: "关于",
      icon: <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10" /><line x1="12" y1="16" x2="12" y2="12" /><line x1="12" y1="8" x2="12.01" y2="8" /></svg>,
      content: <AboutPanel appId="lzy-leisure" appName="休闲时光" />,
    },
    // 局域网访问（仅桌面窗口；浏览器模式无需展示）
    ...(isWeb
      ? []
      : [
          {
            id: "web-access",
            label: "局域网访问",
            icon: <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg>,
            content: (
              <div style={{ padding: "16px 20px", color: "#ddd", fontSize: 14 }}>
                <p style={{ margin: "0 0 4px", color: "#aaa" }}>
                  在浏览器中打开以下地址即可访问（本机或同一局域网的其他设备均可）：
                </p>
                <p style={{ margin: "0 0 16px", color: "#777", fontSize: 13 }}>
                  首次访问需输入下方访问口令
                </p>
                {webUrls.length === 0 ? (
                  <p style={{ color: "#888" }}>正在检测局域网地址…</p>
                ) : (
                  webUrls.map((u) => (
                    <div
                      key={u}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 10,
                        marginBottom: 10,
                        flexWrap: "wrap",
                      }}
                    >
                      <span style={{ color: "#4a90d9", wordBreak: "break-all" }}>{u}</span>
                      <button className="btn" onClick={() => openWeb(u)}>
                        在浏览器打开
                      </button>
                    </div>
                  ))
                )}
                <p style={{ marginTop: 16 }}>
                  访问口令：<b style={{ letterSpacing: 2, color: "#fff" }}>{webPassword}</b>
                </p>
              </div>
            ),
          },
        ]),
  ];

  // 漫画模块 Q 键：返回上一级（reader → 章节列表 / 章节列表 → 书架）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (settingsOpen || activeModule !== "comics") return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key.toLowerCase() !== "q") return;
      // 输入框内不触发，避免打字时误返回
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      e.preventDefault();
      setComicView((v) => {
        if (v.name === "reader") {
          return v.fromSeries ? { name: "series", series: v.fromSeries } : { name: "shelf" };
        }
        if (v.name === "series") return { name: "shelf" };
        return v; // shelf 已在最顶层，无上级
      });
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [settingsOpen, activeModule]);

  const renderComicModule = () => {
    switch (comicView.name) {
      case "series":
        return (
          <ComicChapterView
            series={comicView.series}
            onBack={() => setComicView({ name: "shelf" })}
            onOpenChapter={(comic) =>
              setComicView({ name: "reader", comic, fromSeries: comicView.series })
            }
          />
        );
      case "reader":
        return (
          <ComicReaderView
            comic={comicView.comic}
            onClose={() =>
              setComicView(
                comicView.fromSeries
                  ? { name: "series", series: comicView.fromSeries }
                  : { name: "shelf" },
              )
            }
          />
        );
      default:
        return (
          <ComicShelfView
            onOpenSeries={(comic) => setComicView({ name: "series", series: comic })}
            onOpenComic={(comic) => setComicView({ name: "reader", comic })}
          />
        );
    }
  };

  const renderVideoModule = () => {
    // 播放器（沉浸，最顶层）
    if (playing) {
      return (
        <PlayerView
          video={playing}
          playlist={playlist}
          index={playIndex}
          onSwitch={switchVideo}
          onClose={() => setPlaying(null)}
        />
      );
    }
    // 视频详情页
    if (videoDetail) {
      return (
        <VideoDetailView
          video={videoDetail}
          onBack={() => setVideoDetail(null)}
          onPlay={(v) => void playVideo(v)}
          onChanged={notifyChanged}
        />
      );
    }
    // 剧集详情页
    if (videoSeriesView) {
      return (
        <VideoSeriesView
          seriesId={videoSeriesView.id}
          onBack={() => setVideoSeriesView(null)}
          onPlay={(v) => void playVideo(v)}
          onChanged={notifyChanged}
        />
      );
    }
    // 主区：左侧导航常驻 + 内容（视频墙 / 系列墙 / 资料库页）
    const main =
      videoPage.name === "actors" ? (
        <ActorsView onChanged={notifyChanged} />
      ) : videoPage.name === "tags" ? (
        <TagsView onChanged={notifyChanged} />
      ) : (
        <VideoShelf
          key={shelfKey}
          page={videoPage}
          action={videoAction}
          onActionHandled={() => setVideoAction(null)}
          onOpenVideo={(v) => setVideoDetail(v)}
          onPlayVideo={(v) => void playVideo(v)}
          onOpenSeries={(s) => setVideoSeriesView(s)}
        />
      );
    // 手机端：顶栏 + 底部导航 + 抽屉壳；PC 端：左侧导航常驻 + 内容区
    if (isMobile) {
      return (
        <MobileVideoLayout
          page={videoPage}
          roots={videoRoots}
          onNavigate={setVideoPage}
          onOpenManager={() => setVideoAction("manager")}
          onAddVideo={() => setVideoAction("add")}
        >
          {main}
        </MobileVideoLayout>
      );
    }
    return (
      <div className="v-module">
        <VideoSidebar
          page={videoPage}
          roots={videoRoots}
          onNavigate={setVideoPage}
          onOpenManager={() => setVideoAction("manager")}
          onAddVideo={() => setVideoAction("add")}
        />
        <div className="v-main">{main}</div>
      </div>
    );
  };

  const renderNovelModule = () => {
    if (novelView.name === "reader") {
      return (
        <NovelReaderView
          novel={novelView.novel}
          onClose={() => setNovelView({ name: "shelf" })}
        />
      );
    }
    return <NovelShelfView onOpenNovel={(novel) => setNovelView({ name: "reader", novel })} />;
  };

  const renderStoryModule = () => {
    if (storyView.name === "reader") {
      return (
        <StoryReader
          issue={storyView.issue}
          // 阅读器按 Q/Esc 返回时回到该年期内数（不是直接回年份总览）
          onClose={() => setStoryView({ name: "year", year: storyView.year })}
        />
      );
    }
    return (
      <StoryShelfView
        selectedYear={storyView.name === "year" ? storyView.year : null}
        onSelectYear={(year) =>
          setStoryView(year === null ? { name: "shelf" } : { name: "year", year })
        }
        onOpenIssue={(issue) =>
          setStoryView({
            name: "reader",
            issue,
            year: storyView.name === "year" ? storyView.year : issue.year,
          })
        }
      />
    );
  };

  const renderModule = () => {
    switch (activeModule) {
      case "videos":
        return renderVideoModule();
      case "novels":
        return renderNovelModule();
      case "storyclub":
        return renderStoryModule();
      default:
        return renderComicModule();
    }
  };

  return (
    <AppLayout
      tabBar={moduleBar}
      onTopBarWheel={handleTopBarWheel}
      onSetWindowPin={isWeb ? async () => {} : setWindowPin}
      onSettingsChange={setSettingsOpen}
      settingsTabs={tabs}
    >
      <div className="app-mobile-shell">
        <div className="app-mobile-content">{renderModule()}</div>
        <MobileModuleNav active={activeModule} onChange={setActiveModule} />
      </div>
    </AppLayout>
  );
}

export default App;
