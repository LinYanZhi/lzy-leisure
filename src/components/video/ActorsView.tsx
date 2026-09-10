import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Actor, type ActorInput, type ActorWithCount } from "../../api";

interface Props {
  onChanged: () => void;
  /** 点击演员卡片 → 打开该演员的影片浏览页 */
  onOpenActor: (actor: Actor) => void;
}

function AvatarImage({ actor, ver }: { actor: Actor; ver: number }) {
  const [src, setSrc] = useState<string>();
  useEffect(() => {
    let cancelled = false;
    setSrc(undefined);
    api
      .getActorImageDataUrl(actor.id)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [actor.id, ver]);
  if (!src) return <div className="av-avatar av-avatar-empty">{actor.name.charAt(0)}</div>;
  return <img className="av-avatar" src={src} draggable={false} alt="" />;
}

export default function ActorsView({ onChanged, onOpenActor }: Props) {
  const [actors, setActors] = useState<ActorWithCount[]>([]);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState<Actor | "new" | null>(null);
  const [avatarVer, setAvatarVer] = useState(0);
  const [importing, setImporting] = useState(false);
  const [importMsg, setImportMsg] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    try {
      setActors(await api.listActorsWithCounts());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const remove = async (a: Actor) => {
    if (!confirm(`确定删除演员「${a.name}」？\n（会同时解除与视频的关联）`)) return;
    try {
      await api.deleteActor(a.id);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  // 从 JSON 文件批量导入演员清单（{ name, stage_names, height, cup_size, birthdate, bio }[]）
  const importFromFile = async (file: File) => {
    setImporting(true);
    setImportMsg("");
    setError("");
    try {
      const text = await file.text();
      const raw = JSON.parse(text);
      const list: ActorInput[] = Array.isArray(raw) ? raw : raw.actors;
      if (!Array.isArray(list) || list.length === 0) throw new Error("文件里没有演员数据");
      const n = await api.importActorsBatch(
        list.map((x) => ({
          name: String(x.name ?? "").trim(),
          stage_names: Array.isArray(x.stage_names) ? x.stage_names.map(String) : [],
          height: String(x.height ?? ""),
          cup_size: String(x.cup_size ?? ""),
          birthdate: String(x.birthdate ?? ""),
          bio: String(x.bio ?? ""),
        })),
      );
      setImportMsg(`已导入 ${n} 位演员（重扫视频目录后会自动按文件名匹配演员）`);
      await refresh();
      onChanged();
    } catch (e) {
      setError(`导入失败：${String(e)}`);
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="av-page">
      <div className="av-toolbar">
        <span className="av-count">演员（{actors.length}）</span>
        <div className="av-toolbar-actions">
          <button className="btn" onClick={() => fileInputRef.current?.click()} disabled={importing} title="从 JSON 清单批量导入演员">
            {importing ? "导入中…" : "导入演员"}
          </button>
          <button className="btn btn-primary" onClick={() => setEditing("new")}>＋ 新增演员</button>
        </div>
      </div>
      <input
        ref={fileInputRef}
        type="file"
        accept="application/json,.json"
        style={{ display: "none" }}
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) void importFromFile(f);
          e.target.value = "";
        }}
      />

      {error && <div className="vs-error">{error}</div>}
      {importMsg && <div className="vs-hint">{importMsg}</div>}

      {actors.length === 0 ? (
        <div className="vs-empty">
          <p>演员库为空</p>
          <p className="muted">
            点击右上角「导入演员」从 JSON 清单批量录入，或「新增演员」手动录入
          </p>
        </div>
      ) : (
        <div className="av-grid">
          {actors.map((a) => (
            <div key={a.id} className="av-card" onClick={() => onOpenActor(a)} title={`查看 ${a.name} 的影片`}>
              <div className="av-avatar-wrap">
                <AvatarImage actor={a} ver={avatarVer} />
              </div>
              <div className="av-name">{a.name}</div>
              <div className="av-meta">
                {a.video_count > 0 ? `${a.video_count} 部影片` : "暂无影片"}
              </div>
              <div className="av-meta">
                {[a.stage_names.join(" / "), a.height, a.cup_size].filter(Boolean).join(" · ") || "—"}
              </div>
              <div className="av-actions">
                <button
                  className="av-btn"
                  title="编辑演员资料"
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditing(a);
                  }}
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                    <path d="m15 5 4 4" />
                  </svg>
                </button>
                <button
                  className="av-btn av-danger"
                  title="删除演员"
                  onClick={(e) => {
                    e.stopPropagation();
                    void remove(a);
                  }}
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M3 6h18" />
                    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
                    <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                  </svg>
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {editing && (
        <ActorModal
          actor={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async () => {
            setEditing(null);
            setAvatarVer((n) => n + 1);
            await refresh();
            onChanged();
          }}
        />
      )}
    </div>
  );
}

// ══════════════════════════════════════════════════════════
//  新增 / 编辑演员弹窗
// ══════════════════════════════════════════════════════════

function ActorModal({
  actor,
  onClose,
  onSaved,
}: {
  actor: Actor | null;
  onClose: () => void;
  onSaved: (actorId: string) => Promise<void>;
}) {
  const [name, setName] = useState(actor?.name ?? "");
  const [stageNames, setStageNames] = useState(actor?.stage_names.join(", ") ?? "");
  const [height, setHeight] = useState(actor?.height ?? "");
  const [cupSize, setCupSize] = useState(actor?.cup_size ?? "");
  const [birthdate, setBirthdate] = useState(actor?.birthdate ?? "");
  const [bio, setBio] = useState(actor?.bio ?? "");
  const [avatarFile, setAvatarFile] = useState<File | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  const save = async () => {
    if (!name.trim()) {
      setError("请填写姓名");
      return;
    }
    setSaving(true);
    setError("");
    try {
      const saved = await api.saveActor({
        id: actor?.id,
        name: name.trim(),
        stage_names: stageNames.split(",").map((s) => s.trim()).filter(Boolean),
        height: height.trim(),
        cup_size: cupSize.trim(),
        birthdate: birthdate.trim(),
        bio: bio.trim(),
      });
      if (avatarFile) {
        const reader = new FileReader();
        const dataUrl = await new Promise<string>((resolve, reject) => {
          reader.onload = () => resolve(String(reader.result));
          reader.onerror = () => reject(new Error("读取图片失败"));
          reader.readAsDataURL(avatarFile);
        });
        await api.uploadActorImage(saved.id, dataUrl);
      }
      await onSaved(saved.id);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal">
        <div className="modal-header">
          <h3>{actor ? "编辑演员" : "新增演员"}</h3>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>
        <div className="modal-body av-form">
          <label className="pv-field">
            <span>姓名 <em>*</em></span>
            <input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          <label className="pv-field">
            <span>别名（逗号分隔）</span>
            <input value={stageNames} onChange={(e) => setStageNames(e.target.value)} placeholder="如：三上悠亚, Yua Mikami" />
          </label>
          <div className="pv-field-row">
            <label className="pv-field">
              <span>身高</span>
              <input value={height} onChange={(e) => setHeight(e.target.value)} placeholder="158cm" />
            </label>
            <label className="pv-field">
              <span>罩杯</span>
              <input value={cupSize} onChange={(e) => setCupSize(e.target.value)} placeholder="D" />
            </label>
            <label className="pv-field">
              <span>出生日期</span>
              <input value={birthdate} onChange={(e) => setBirthdate(e.target.value)} placeholder="1993-08-16" />
            </label>
          </div>
          <label className="pv-field">
            <span>简介</span>
            <textarea value={bio} onChange={(e) => setBio(e.target.value)} rows={3} />
          </label>
          <label className="pv-field">
            <span>头像</span>
            <input
              type="file"
              accept="image/*"
              onChange={(e) => setAvatarFile(e.target.files?.[0] ?? null)}
            />
            {avatarFile && <span className="pv-hint">已选择：{avatarFile.name}</span>}
          </label>
          {error && <div className="pv-error">{error}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={save} disabled={saving}>
            {saving ? "保存中…" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
