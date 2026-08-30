import { useCallback, useEffect, useState } from "react";
import { api, type Tag, type TagGroup } from "../../api";

interface Props {
  onChanged: () => void;
}

const UNGROUPED_ID = "__ungrouped__";

export default function TagsView({ onChanged }: Props) {
  const [groups, setGroups] = useState<TagGroup[]>([]);
  const [tags, setTags] = useState<Tag[]>([]);
  const [selectedGroup, setSelectedGroup] = useState<string>("");
  const [error, setError] = useState("");
  const [groupModal, setGroupModal] = useState(false);
  const [tagModal, setTagModal] = useState<Tag | "new" | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [g, t] = await Promise.all([api.listTagGroups(), api.listTags()]);
      setGroups(g);
      setTags(t);
      setSelectedGroup((cur) => (cur && (g.some((x) => x.id === cur) || cur === UNGROUPED_ID) ? cur : g[0]?.id ?? UNGROUPED_ID));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const visibleTags = tags.filter((t) =>
    selectedGroup === UNGROUPED_ID ? t.group_id === "" : t.group_id === selectedGroup,
  );

  const deleteGroup = async (g: TagGroup) => {
    if (!confirm(`确定删除标签组「${g.name}」？\n（组内标签保留，但归属清空）`)) return;
    try {
      await api.deleteTagGroup(g.id);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const deleteTag = async (t: Tag) => {
    if (!confirm(`确定删除标签「${t.name}」？`)) return;
    try {
      await api.deleteTag(t.id);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="tv-page">
      <div className="tv-groups">
        <div className="tv-panel-head">
          <span className="tv-panel-title">标签组</span>
          <button className="btn btn-sm" onClick={() => setGroupModal(true)}>＋ 新建</button>
        </div>
        <div
          className={`tv-group-item${selectedGroup === UNGROUPED_ID ? " active" : ""}`}
          onClick={() => setSelectedGroup(UNGROUPED_ID)}
        >
          <span>未分组</span>
          <span className="tv-count">{tags.filter((t) => t.group_id === "").length}</span>
        </div>
        {groups.map((g) => (
          <div
            key={g.id}
            className={`tv-group-item${selectedGroup === g.id ? " active" : ""}`}
            onClick={() => setSelectedGroup(g.id)}
          >
            <span className="tv-group-name">{g.name}</span>
            <span className="tv-count">{tags.filter((t) => t.group_id === g.id).length}</span>
            <button
              className="tv-group-del"
              title="删除标签组"
              onClick={(e) => {
                e.stopPropagation();
                void deleteGroup(g);
              }}
            >
              ×
            </button>
          </div>
        ))}
      </div>

      <div className="tv-tags">
        <div className="tv-panel-head">
          <span className="tv-panel-title">
            标签
            <span className="tv-panel-sub">
              {selectedGroup === UNGROUPED_ID ? "未分组" : groups.find((g) => g.id === selectedGroup)?.name}
            </span>
          </span>
          <button className="btn btn-sm" onClick={() => setTagModal("new")}>＋ 新建标签</button>
        </div>

        {error && <div className="vs-error">{error}</div>}

        {visibleTags.length === 0 ? (
          <div className="vs-empty">
            <p>该组暂无标签</p>
            <p className="muted">点击右上角「新建标签」创建</p>
          </div>
        ) : (
          <div className="tv-tag-list">
            {visibleTags.map((t) => (
              <div key={t.id} className="tv-tag-item">
                <span className="tv-color-dot" style={{ background: t.color }} />
                <span className="tv-tag-name">{t.name}</span>
                <button className="tv-group-del" title="删除标签" onClick={() => void deleteTag(t)}>×</button>
              </div>
            ))}
          </div>
        )}
      </div>

      {groupModal && (
        <GroupModal
          onClose={() => setGroupModal(false)}
          onSaved={async () => {
            setGroupModal(false);
            await refresh();
            onChanged();
          }}
        />
      )}
      {tagModal && (
        <TagModal
          tag={tagModal === "new" ? null : tagModal}
          groups={groups}
          defaultGroupId={selectedGroup === UNGROUPED_ID ? "" : selectedGroup}
          onClose={() => setTagModal(null)}
          onSaved={async () => {
            setTagModal(null);
            await refresh();
            onChanged();
          }}
        />
      )}
    </div>
  );
}

// ══════════════════════════════════════════════════════════
//  新建标签组弹窗
// ══════════════════════════════════════════════════════════

function GroupModal({ onClose, onSaved }: { onClose: () => void; onSaved: () => Promise<void> }) {
  const [name, setName] = useState("");
  const [error, setError] = useState("");

  const save = async () => {
    if (!name.trim()) {
      setError("请填写标签组名称");
      return;
    }
    try {
      await api.saveTagGroup(name.trim());
      await onSaved();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal">
        <div className="modal-header">
          <h3>新建标签组</h3>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>
        <div className="modal-body">
          <label className="pv-field">
            <span>组名</span>
            <input value={name} onChange={(e) => setName(e.target.value)} autoFocus onKeyDown={(e) => { if (e.key === "Enter") void save(); }} />
          </label>
          {error && <div className="pv-error">{error}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={save}>保存</button>
        </div>
      </div>
    </div>
  );
}

// ══════════════════════════════════════════════════════════
//  新建 / 编辑标签弹窗
// ══════════════════════════════════════════════════════════

function TagModal({
  tag,
  groups,
  defaultGroupId,
  onClose,
  onSaved,
}: {
  tag: Tag | null;
  groups: TagGroup[];
  defaultGroupId: string;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [name, setName] = useState(tag?.name ?? "");
  const [color, setColor] = useState(tag?.color ?? "#8a8a8a");
  const [groupId, setGroupId] = useState(tag?.group_id ?? defaultGroupId);
  const [error, setError] = useState("");

  const save = async () => {
    if (!name.trim()) {
      setError("请填写标签名称");
      return;
    }
    try {
      await api.saveTag({ id: tag?.id, name: name.trim(), color, group_id: groupId });
      await onSaved();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal">
        <div className="modal-header">
          <h3>{tag ? "编辑标签" : "新建标签"}</h3>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>
        <div className="modal-body">
          <label className="pv-field">
            <span>名称</span>
            <input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          <div className="pv-field-row">
            <label className="pv-field">
              <span>颜色</span>
              <input type="color" value={color} onChange={(e) => setColor(e.target.value)} />
            </label>
            <label className="pv-field">
              <span>所属组</span>
              <select value={groupId} onChange={(e) => setGroupId(e.target.value)}>
                <option value="">未分组</option>
                {groups.map((g) => (
                  <option key={g.id} value={g.id}>{g.name}</option>
                ))}
              </select>
            </label>
          </div>
          {error && <div className="pv-error">{error}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={save}>保存</button>
        </div>
      </div>
    </div>
  );
}
