import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  type Actor,
  type SeriesDetail,
  type Tag,
  type TagGroup,
  type Video,
} from "../../api";
import CoverImage from "./CoverImage";
import { VIDEO_KINDS } from "./kinds";
import { ensureVideoCover, probeVideoMeta } from "./videoMedia";

interface Props {
  video: Video;
  onBack: () => void;
  onPlay: (video: Video) => void;
  onChanged: () => void;
}

/**
 * 视频详情页：大封面 + 元数据编辑 + 演员/标签 + 所属剧集快速切换。
 * 播放进入沉浸式播放器；保存/删除后通知 App 刷新列表。
 */
export default function VideoDetailView({ video, onBack, onPlay, onChanged }: Props) {
  const [cur, setCur] = useState<Video>(video);
  const [actors, setActors] = useState<Actor[]>([]);
  const [tags, setTags] = useState<Tag[]>([]);
  const [groups, setGroups] = useState<TagGroup[]>([]);
  const [seriesDetail, setSeriesDetail] = useState<SeriesDetail | null>(null);
  const [coverVer, setCoverVer] = useState(0);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // 编辑字段（切换视频时重置）
  const [title, setTitle] = useState(video.title);
  const [description, setDescription] = useState(video.description);
  const [licensePlate, setLicensePlate] = useState(video.license_plate);
  const [year, setYear] = useState(video.year);
  const [rating, setRating] = useState(video.rating);
  const [kindIds, setKindIds] = useState<string[]>(video.kinds);
  const [actorIds, setActorIds] = useState<string[]>(video.actors);
  const [tagIds, setTagIds] = useState<string[]>(video.tags);

  useEffect(() => {
    setTitle(cur.title);
    setDescription(cur.description);
    setLicensePlate(cur.license_plate);
    setYear(cur.year);
    setRating(cur.rating);
    setKindIds(cur.kinds);
    setActorIds(cur.actors);
    setTagIds(cur.tags);
    setSaved(false);
    setError("");
  }, [cur]);

  // 加载演员/标签/所属剧集
  useEffect(() => {
    Promise.all([api.listActors(), api.listTags(), api.listTagGroups()])
      .then(([a, t, g]) => {
        setActors(a);
        setTags(t);
        setGroups(g);
      })
      .catch((e) => setError(String(e)));
  }, []);

  // 元数据补全：缺时长/分辨率时前端读取并写回（仅打开时补一次）
  useEffect(() => {
    if (cur.duration) return;
    let cancelled = false;
    probeVideoMeta(cur)
      .then((patch) => {
        if (cancelled || !patch) return;
        setCur((prev) => ({
          ...prev,
          duration: patch.duration || prev.duration,
          frame_width: patch.frameWidth ?? prev.frame_width,
          frame_height: patch.frameHeight ?? prev.frame_height,
        }));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cur.id]);

  useEffect(() => {
    if (!cur.series_id) {
      setSeriesDetail(null);
      return;
    }
    let cancelled = false;
    api
      .getSeries(cur.series_id)
      .then((d) => {
        if (!cancelled) setSeriesDetail(d);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [cur.series_id]);

  const toggleId = useCallback((list: string[], id: string) => {
    return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
  }, []);

  const save = async () => {
    setError("");
    setSaved(false);
    try {
      await api.updateVideo({
        id: cur.id,
        title,
        description,
        license_plate: licensePlate,
        year,
        rating,
        kinds: kindIds,
        actor_ids: actorIds,
        tag_ids: tagIds,
      });
      setSaved(true);
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  const openFolder = async () => {
    try {
      await api.openVideoFolder(cur.id);
    } catch (e) {
      setError(String(e));
    }
  };

  const regenerate = async () => {
    setError("");
    try {
      const ok = await ensureVideoCover(cur, true);
      if (ok) setCoverVer((n) => n + 1);
      else setError("无法生成封面（浏览器不支持该视频格式）");
    } catch (e) {
      setError(String(e));
    }
  };

  const uploadCover = (file: File) => {
    const reader = new FileReader();
    reader.onload = async () => {
      try {
        await api.uploadCover(cur.id, String(reader.result));
        setCoverVer((n) => n + 1);
      } catch (e) {
        setError(String(e));
      }
    };
    reader.readAsDataURL(file);
  };

  const remove = async () => {
    if (!confirm(`确定删除「${cur.title}」的记录？\n（不会删除本地文件）`)) return;
    try {
      await api.deleteVideo(cur.id);
      onChanged();
      onBack();
    } catch (e) {
      setError(String(e));
    }
  };

  const res = cur.frame_width && cur.frame_height ? `${cur.frame_width}×${cur.frame_height}` : "";
  const metaItems: [string, string][] = [
    ["时长", cur.duration],
    ["分辨率", res],
    ["帧率", cur.fps ? `${cur.fps.toFixed(0)} fps` : ""],
    ["大小", cur.file_size],
    ["格式", cur.file_type],
  ];

  return (
    <div className="vdt-page">
      <div className="vdt-topbar">
        <button className="btn btn-sm" onClick={onBack}>← 返回</button>
        <div className="vdt-title" title={cur.title}>{title || cur.title}</div>
        <div className="vdt-topbar-actions">
          <button className="btn btn-sm btn-primary" onClick={() => onPlay(cur)}>
            ▶ 播放
          </button>
          <button className="btn btn-sm btn-danger" onClick={remove}>删除</button>
        </div>
      </div>

      <div className="vdt-body">
        <div className="vdt-hero">
          <div className="vdt-cover">
            <CoverImage videoId={cur.id} version={coverVer} className="vdt-cover-img" />
          </div>

          <div className="vdt-info">
            <label className="vdt-field">
              <span>标题</span>
              <input value={title} onChange={(e) => setTitle(e.target.value)} />
            </label>

            <div className="vdt-field-row">
              <label className="vdt-field">
                <span>年份</span>
                <input value={year} onChange={(e) => setYear(e.target.value)} placeholder="如 2020" />
              </label>
              <label className="vdt-field">
                <span>评分（0-10）</span>
                <input
                  type="number"
                  min={0}
                  max={10}
                  step={0.5}
                  value={rating || ""}
                  onChange={(e) => setRating(Number(e.target.value) || 0)}
                  placeholder="0-10"
                />
              </label>
            </div>

            <div className="vdt-field">
              <span>种类</span>
              <div className="vdt-chips">
                {VIDEO_KINDS.map((k) => (
                  <button
                    key={k.id}
                    className={`vdt-chip${kindIds.includes(k.id) ? " active" : ""}`}
                    onClick={() => setKindIds((prev) => toggleId(prev, k.id))}
                  >
                    {k.label}
                  </button>
                ))}
              </div>
            </div>

            <div className="vdt-meta-grid">
              {metaItems
                .filter(([, v]) => v)
                .map(([k, v]) => (
                  <span key={k} className="vdt-meta-item">
                    {k}：<b>{v}</b>
                  </span>
                ))}
            </div>

            <label className="vdt-field">
              <span>车牌号</span>
              <input value={licensePlate} onChange={(e) => setLicensePlate(e.target.value)} placeholder="JUL-000" />
            </label>
            <label className="vdt-field">
              <span>简介</span>
              <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={3} />
            </label>

            <div className="vdt-field">
              <span>演员</span>
              <div className="vdt-chips">
                {actors.length === 0 && <span className="vdt-hint">暂无演员，可到「演员」页添加</span>}
                {actors.map((a) => (
                  <button
                    key={a.id}
                    className={`vdt-chip${actorIds.includes(a.id) ? " active" : ""}`}
                    onClick={() => setActorIds((prev) => toggleId(prev, a.id))}
                  >
                    {a.name}
                  </button>
                ))}
              </div>
            </div>

            {groups.length > 0 && (
              <div className="vdt-field">
                <span>标签</span>
                {groups.map((g) => {
                  const gtags = tags.filter((t) => t.group_id === g.id);
                  if (gtags.length === 0) return null;
                  return (
                    <div className="vdt-chip-group" key={g.id}>
                      <span className="vdt-chip-group-label">{g.name}</span>
                      <div className="vdt-chips">
                        {gtags.map((t) => (
                          <button
                            key={t.id}
                            className={`vdt-chip${tagIds.includes(t.id) ? " active" : ""}`}
                            style={tagIds.includes(t.id) ? { borderColor: t.color, color: t.color } : undefined}
                            onClick={() => setTagIds((prev) => toggleId(prev, t.id))}
                          >
                            {t.name}
                          </button>
                        ))}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            <div className="vdt-actions">
              <button className="btn btn-primary" onClick={save}>
                {saved ? "已保存 ✓" : "保存修改"}
              </button>
              <button className="btn" onClick={openFolder}>打开目录</button>
              <button className="btn" onClick={regenerate}>重生成封面</button>
              <button className="btn" onClick={() => fileInputRef.current?.click()}>上传封面</button>
            </div>
            <input
              ref={fileInputRef}
              type="file"
              accept="image/*"
              style={{ display: "none" }}
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) uploadCover(f);
                e.target.value = "";
              }}
            />
            <div className="vdt-path" title={cur.path}>{cur.path}</div>
            {error && <div className="vs-error">{error}</div>}
          </div>
        </div>

        {/* ── 所属剧集（同剧集快速切换） ── */}
        {seriesDetail && (
          <div className="vdt-series">
            <div className="vdt-series-title">
              {seriesDetail.series.title} · 共 {seriesDetail.videos.length} 集
            </div>
            <div className="vdt-series-list">
              {seriesDetail.videos.map((v) => (
                <div
                  key={v.id}
                  className={`vdt-series-ep${v.id === cur.id ? " active" : ""}`}
                  onClick={() => setCur(v)}
                  title={v.title}
                >
                  <div className="vdt-series-ep-cover">
                    <CoverImage videoId={v.id} className="vdt-series-ep-img" />
                  </div>
                  <div className="vdt-series-ep-title">
                    {v.episode ? `${v.episode} · ` : ""}{v.title}
                  </div>
                  <div className="vdt-series-ep-meta">
                    {v.duration}
                    {v.progress > 0 && v.duration ? " · 有进度" : ""}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
