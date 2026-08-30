import { useCallback, useEffect, useState } from "react";
import { api, type SeriesDetail, type Video } from "../../api";
import CoverImage from "./CoverImage";
import { probeVideoMeta } from "./videoMedia";

interface Props {
  seriesId: string;
  onBack: () => void;
  onPlay: (video: Video) => void;
  onChanged: () => void;
}

/** duration（HH:MM:SS 或 MM:SS）→ 秒 */
function durToSec(dur: string): number {
  const parts = dur.split(":").map((x) => parseFloat(x) || 0);
  let sec = 0;
  for (const p of parts) sec = sec * 60 + p;
  return sec;
}

export default function VideoSeriesView({ seriesId, onBack, onPlay, onChanged }: Props) {
  const [detail, setDetail] = useState<SeriesDetail | null>(null);
  const [error, setError] = useState("");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    try {
      const d = await api.getSeries(seriesId);
      setDetail(d);
      setTitle(d.series.title);
      setDescription(d.series.description);
    } catch (e) {
      setError(String(e));
    }
  }, [seriesId]);

  useEffect(() => {
    load();
  }, [load]);

  // 元数据补全：缺时长/分辨率的视频前端读取并写回（收敛：补完后缺失列表为空）
  useEffect(() => {
    if (!detail) return;
    const missing = detail.videos.filter((v) => !v.duration);
    if (missing.length === 0) return;
    let cancelled = false;
    for (const v of missing) {
      probeVideoMeta(v)
        .then((patch) => {
          if (cancelled || !patch) return;
          setDetail((d) =>
            d
              ? {
                  ...d,
                  videos: d.videos.map((x) =>
                    x.id === v.id
                      ? {
                          ...x,
                          duration: patch.duration || x.duration,
                          frame_width: patch.frameWidth ?? x.frame_width,
                          frame_height: patch.frameHeight ?? x.frame_height,
                        }
                      : x,
                  ),
                }
              : d,
          );
        })
        .catch(() => {});
    }
    return () => {
      cancelled = true;
    };
  }, [detail]);

  const save = async () => {
    setError("");
    setSaved(false);
    try {
      await api.updateSeries(seriesId, title, description);
      setSaved(true);
      setDetail((d) => (d ? { ...d, series: { ...d.series, title, description } } : d));
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  if (!detail) {
    return (
      <div className="vsv-page">
        <div className="vsv-topbar">
          <button className="btn btn-sm" onClick={onBack}>← 返回</button>
        </div>
        {error ? <div className="vsv-error">{error}</div> : <div className="vsv-empty">加载中…</div>}
      </div>
    );
  }

  const s = detail.series;

  return (
    <div className="vsv-page">
      <div className="vsv-topbar">
        <button className="btn btn-sm" onClick={onBack}>← 返回</button>
        <div className="vsv-title" title={s.title}>{title || s.title}</div>
        <button className="btn btn-sm btn-primary" onClick={save}>
          {saved ? "已保存 ✓" : "保存"}
        </button>
      </div>

      <div className="vsv-body">
        <div className="vsv-header">
          <div className="vsv-cover">
            {s.cover_video_id ? (
              <CoverImage videoId={s.cover_video_id} className="vsv-cover-img" />
            ) : (
              <div className="vsv-cover-empty">无封面</div>
            )}
          </div>
          <div className="vsv-info">
            <label className="pv-field">
              <span>标题</span>
              <input value={title} onChange={(e) => setTitle(e.target.value)} />
            </label>
            <label className="pv-field">
              <span>简介</span>
              <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={3} />
            </label>
            <div className="vsv-meta">共 {detail.videos.length} 集</div>
          </div>
        </div>

        <div className="vsv-section-title">剧集列表（按标题自然排序）</div>
        <div className="vsv-episodes">
          {detail.videos.length === 0 ? (
            <div className="vsv-empty">该剧集下还没有视频</div>
          ) : (
            detail.videos.map((v, i) => (
              <div key={v.id} className="vsv-ep" onClick={() => onPlay(v)} title={v.title}>
                <span className="vsv-ep-index" title={v.episode ? "集数" : "序号"}>
                  {v.episode || i + 1}
                </span>
                <div className="vsv-ep-cover">
                  <CoverImage videoId={v.id} className="vsv-ep-img" />
                  {v.progress > 0 && v.duration && (
                    <div className="vs-progress-bar" title="播放进度">
                      <div
                        className="vs-progress-fill"
                        style={{ width: `${Math.min(100, (v.progress / durToSec(v.duration)) * 100)}%` }}
                      />
                    </div>
                  )}
                </div>
                <div className="vsv-ep-info">
                  <div className="vsv-ep-title">{v.title}</div>
                  <div className="vsv-ep-meta">
                    {[v.duration, v.file_size].filter(Boolean).join(" · ") || v.file_type}
                  </div>
                </div>
                <button className="btn btn-sm" onClick={(e) => { e.stopPropagation(); onPlay(v); }}>
                  播放
                </button>
              </div>
            ))
          )}
        </div>
      </div>

      {error && <div className="vsv-error">{error}</div>}
    </div>
  );
}
