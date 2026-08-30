import type { VideoRootDir } from "../../api";
import { VIDEO_KINDS } from "./kinds";

/** 视频模块页面导航（左侧导航驱动主内容区） */
export type VideoPage =
  | { name: "all" }
  | { name: "series" }
  | { name: "kind"; kind: string }
  | { name: "root"; path: string }
  | { name: "actors" }
  | { name: "tags" };

interface Props {
  page: VideoPage;
  roots: VideoRootDir[];
  onNavigate: (p: VideoPage) => void;
  onOpenManager: () => void;
  onAddVideo: () => void;
}

function NavItem({
  active,
  label,
  count,
  onClick,
}: {
  active: boolean;
  label: string;
  count?: number;
  onClick: () => void;
}) {
  return (
    <div
      className={`v-nav-item${active ? " active" : ""}`}
      onClick={onClick}
      title={label}
    >
      <span className="v-nav-label">{label}</span>
      {count !== undefined && <span className="v-nav-count">{count}</span>}
    </div>
  );
}

function NavGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="v-nav-group">
      <div className="v-nav-group-title">{title}</div>
      {children}
    </div>
  );
}

/** 视频模块左侧导航：浏览 / 种类 / 导入路径 / 资料库 */
export default function VideoSidebar({ page, roots, onNavigate, onOpenManager, onAddVideo }: Props) {
  const totalVideos = roots.reduce((n, r) => n + r.video_count, 0);

  return (
    <div className="v-sidebar">
      <NavGroup title="浏览">
        <NavItem
          active={page.name === "all"}
          label="全部视频"
          count={totalVideos}
          onClick={() => onNavigate({ name: "all" })}
        />
        <NavItem
          active={page.name === "series"}
          label="剧集"
          onClick={() => onNavigate({ name: "series" })}
        />
      </NavGroup>

      <NavGroup title="种类">
        {VIDEO_KINDS.map((k) => (
          <NavItem
            key={k.id}
            active={page.name === "kind" && page.kind === k.id}
            label={k.label}
            onClick={() => onNavigate({ name: "kind", kind: k.id })}
          />
        ))}
      </NavGroup>

      <NavGroup title="导入路径">
        {roots.length === 0 ? (
          <div className="v-nav-empty">暂无导入路径</div>
        ) : (
          roots.map((r) => (
            <NavItem
              key={r.path}
              active={page.name === "root" && page.path === r.path}
              label={r.name}
              count={r.video_count}
              onClick={() => onNavigate({ name: "root", path: r.path })}
            />
          ))
        )}
      </NavGroup>

      <NavGroup title="资料库">
        <NavItem
          active={page.name === "actors"}
          label="演员"
          onClick={() => onNavigate({ name: "actors" })}
        />
        <NavItem
          active={page.name === "tags"}
          label="标签"
          onClick={() => onNavigate({ name: "tags" })}
        />
      </NavGroup>

      <div className="v-sidebar-footer">
        <button className="btn-ghost v-sidebar-btn" onClick={onOpenManager} title="管理导入的视频目录 / 创建剧集">
          视频源管理
        </button>
        <button className="btn-ghost v-sidebar-btn" onClick={onAddVideo} title="导入单个视频文件">
          + 添加视频
        </button>
      </div>
    </div>
  );
}
