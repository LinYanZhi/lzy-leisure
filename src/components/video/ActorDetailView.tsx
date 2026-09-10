import { useCallback, useEffect, useState } from "react";
import { api, type Actor, type Video } from "../../api";
import CoverImage from "./CoverImage";

interface Props {
  actor: Actor;
  onBack: () => void;
  onOpenVideo: (video: Video) => void;
  onPlayVideo: (video: Video) => void;
}

/** 秒数格式化为 mm:ss / hh:mm:ss（与书架一致） */
function fmtSec(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = m < 10 ? `0${m}` : `${m}`;
  const ss = r < 10 ? `0${r}` : `${r}`;
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

function durToSec(dur: string): number {
  const parts = dur.split(":").map((x) => parseFloat(x) || 0);
  let sec = 0;
  for (const p of parts) sec = sec * 60 + p;
  return sec;
}

/**
 * 演员详情页：演员资料 + TA 名下全部影片网格。
 * 核心交互：点卡片 → 视频详情；点播放 → 直接播放。移动端/网页端共用同一视图。
 */
export default function ActorDetailView({ actor, onBack, onOpenVideo, onPlayVideo }: Props) {
  const [videos, setVideos] = useState<Video[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [avatarSrc, setAvatarSrc] = useState<string>();

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      setVideos(await api.listVideos({ actor_ids: [actor.id] }));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [actor.id]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    setAvatarSrc(undefined);
    api
      .getActorImageDataUrl(actor.id)
      .then((url) => {
        if (!cancelled) setAvatarSrc(url);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [actor.id]);

  const metaItems = [
    actor.stage_names.length > 0 ? actor.stage_names.join(" / ") : "",
    actor.height,
    actor.cup_size,
    actor.birthdate,
  ].filter(Boolean);

  return (
    <div className="adv-page">
      {/* ── 顶栏 ── */}
      <div className="adv-topbar">
        <button className="btn btn-sm" onClick={onBack}>← 返回</button>
        <div className="adv-title" title={actor.name}>{actor.name}</div>
        <div className="adv-count">{videos.length} 部影片</div>
      </div>

      {/* ── 演员资料 ── */}
      <div className="adv-hero">
        {avatarSrc ? (
          <img className="adv-avatar" src={avatarSrc} alt="" draggable={false} />
        ) : (
          <div className="adv-avatar adv-avatar-empty">{actor.name.charAt(0)}</div>
        )}
        <div className="adv-info">
          <div className="adv-name">{actor.name}</div>
          {metaItems.length > 0 && (
            <div className="adv-meta">{metaItems.join(" · ")}</div>
          )}
          {actor.bio && <div className="adv-bio">{actor.bio}</div>}
          <div className="adv-hint">点击影片卡片查看详情，悬停可快速播放</div>
        </div>
      </div>

      {error && <div className="vs-error">{error}</div>}

      {/* ── 影片网格 ── */}
      {loading ? (
        <div className="vs-empty">加载中…</div>
      ) : videos.length === 0 ? (
        <div className="vs-empty">
          <p>该演员名下暂无影片</p>
          <p className="muted">可在「视频源管理」重新扫描目录，自动按文件名匹配演员；或到视频详情页手动关联</p>
        </div>
      ) : (
        <div className="vs-grid adv-grid">
          {videos.map((v) => {
            const meta = [v.duration, v.license_plate, v.year].filter(Boolean).join(" · ") || v.file_type;
            return (
              <div key={v.id} className="vs-card" onClick={() => onOpenVideo(v)} title={v.title}>
                <div className="vs-cover-wrap">
                  <CoverImage videoId={v.id} className="vs-cover" />
                  {v.subtitle_path && (
                    <span className="vs-sub-badge" title="有同名字幕文件">字幕</span>
                  )}
                  {v.progress > 0 && v.duration && (
                    <div className="vs-progress-bar" title={`已播放 ${fmtSec(v.progress)} / ${v.duration}`}>
                      <div
                        className="vs-progress-fill"
                        style={{ width: `${Math.min(100, (v.progress / durToSec(v.duration)) * 100)}%` }}
                      />
                    </div>
                  )}
                  <div className="vs-card-hover">
                    <button
                      className="vs-card-btn"
                      title="播放"
                      onClick={(e) => {
                        e.stopPropagation();
                        onPlayVideo(v);
                      }}
                    >
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <polygon points="5 3 19 12 5 21 5 3" />
                      </svg>
                    </button>
                  </div>
                </div>
                <div className="vs-title" title={v.title}>{v.title}</div>
                <div className="vs-meta">{meta}</div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
