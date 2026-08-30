import { useCallback, useEffect, useState } from "react";
import { api, type Actor } from "../../api";

interface Props {
  onChanged: () => void;
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

export default function ActorsView({ onChanged }: Props) {
  const [actors, setActors] = useState<Actor[]>([]);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState<Actor | "new" | null>(null);
  const [avatarVer, setAvatarVer] = useState(0);

  const refresh = useCallback(async () => {
    try {
      setActors(await api.listActors());
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

  return (
    <div className="av-page">
      <div className="av-toolbar">
        <span className="av-count">演员（{actors.length}）</span>
        <button className="btn btn-primary" onClick={() => setEditing("new")}>＋ 新增演员</button>
      </div>

      {error && <div className="vs-error">{error}</div>}

      {actors.length === 0 ? (
        <div className="vs-empty">
          <p>演员库为空</p>
          <p className="muted">点击右上角「新增演员」录入演员信息</p>
        </div>
      ) : (
        <div className="av-grid">
          {actors.map((a) => (
            <div key={a.id} className="av-card">
              <div className="av-avatar-wrap" onClick={() => setEditing(a)} title="点击编辑">
                <AvatarImage actor={a} ver={avatarVer} />
              </div>
              <div className="av-name" onClick={() => setEditing(a)} title="点击编辑">{a.name}</div>
              <div className="av-meta">
                {[a.stage_names.join(" / "), a.height, a.cup_size].filter(Boolean).join(" · ") || "—"}
              </div>
              <button className="av-delete" title="删除演员" onClick={() => void remove(a)}>
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M3 6h18" />
                  <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
                  <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                </svg>
              </button>
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
