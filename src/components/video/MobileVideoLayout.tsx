import { useState } from "react";
import type { VideoRootDir } from "../../api";
import VideoSidebar, { type VideoPage } from "./VideoSidebar";

interface Props {
  page: VideoPage;
  roots: VideoRootDir[];
  onNavigate: (p: VideoPage) => void;
  onOpenManager: () => void;
  onAddVideo: () => void;
  children: React.ReactNode;
}

/** 顶部二级导航项：手机端一屏可达的常用页面 */
const BOTTOM_NAV: { page: VideoPage; label: string }[] = [
  { page: { name: "all" }, label: "全部" },
  { page: { name: "series" }, label: "剧集" },
  { page: { name: "actors" }, label: "演员" },
  { page: { name: "tags" }, label: "标签" },
];

/**
 * 手机端视频模块壳：内容顶部二级导航横条（全部/剧集/演员/标签/菜单）→ 内容区。
 * 完整导航（种类 / 导入路径 / 资料库 / 视频源管理）在「菜单」抽屉里复用 VideoSidebar。
 * 模块切换（漫画/视频/小说/故事会）由 App 级底部导航承担，本壳不再占用底部。
 */
export default function MobileVideoLayout({
  page,
  roots,
  onNavigate,
  onOpenManager,
  onAddVideo,
  children,
}: Props) {
  const [drawerOpen, setDrawerOpen] = useState(false);

  const closeDrawer = () => setDrawerOpen(false);
  const go = (p: VideoPage) => {
    onNavigate(p);
    closeDrawer();
  };

  return (
    <div className="v-module v-module-mobile">
      <div className="vm-secnav">
        {BOTTOM_NAV.map((item) => (
          <button
            key={item.label}
            className={`vm-sec-item${page.name === item.page.name ? " active" : ""}`}
            onClick={() => go(item.page)}
          >
            {item.label}
          </button>
        ))}
        <button
          className={`vm-sec-item${drawerOpen ? " active" : ""}`}
          onClick={() => setDrawerOpen(true)}
        >
          菜单
        </button>
      </div>

      <div className="vm-body">{children}</div>

      {drawerOpen && (
        <div className="vm-drawer-overlay" onClick={closeDrawer}>
          <div className="vm-drawer" onClick={(e) => e.stopPropagation()}>
            <div className="vm-drawer-head">
              <span className="vm-drawer-title">视频导航</span>
              <button className="vm-icon-btn" onClick={closeDrawer} title="关闭" aria-label="关闭">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
            </div>
            <VideoSidebar
              page={page}
              roots={roots}
              onNavigate={go}
              onOpenManager={() => {
                onOpenManager();
                closeDrawer();
              }}
              onAddVideo={() => {
                onAddVideo();
                closeDrawer();
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
