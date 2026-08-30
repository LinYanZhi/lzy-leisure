//! 视频库——演员与标签组/标签。
//! 原 db/videos.rs 拆分（数据模型与视频见 videos.rs，剧集见 videos_series）。
use super::{Actor, TagGroup, Tag};
use crate::db::lock;
use rusqlite::{params, OptionalExtension};

// ══════════════════════════════════════════════════════════
//  演员
// ══════════════════════════════════════════════════════════

pub fn list_actors() -> Result<Vec<Actor>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM actors ORDER BY sort_order, created_at")
        .map_err(|e| format!("查询演员失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Actor {
                id: row.get(0)?,
                name: row.get(1)?,
                stage_names: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                height: row.get(3)?,
                cup_size: row.get(4)?,
                birthdate: row.get(5)?,
                bio: row.get(6)?,
                images: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
                sort_order: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| format!("读取演员失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("解析演员失败: {e}"))?);
    }
    Ok(out)
}

pub fn get_actor(actor_id: &str) -> Result<Option<Actor>, String> {
    let conn = lock()?;
    conn.query_row(
        "SELECT * FROM actors WHERE id = ?1",
        params![actor_id],
        |row| {
            Ok(Actor {
                id: row.get(0)?,
                name: row.get(1)?,
                stage_names: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                height: row.get(3)?,
                cup_size: row.get(4)?,
                birthdate: row.get(5)?,
                bio: row.get(6)?,
                images: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
                sort_order: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("查询演员失败: {e}"))
}

pub fn upsert_actor(actor: &Actor) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        r#"INSERT INTO actors (id, name, stage_names, height, cup_size, birthdate, bio, images, sort_order, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
           ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             stage_names = excluded.stage_names,
             height = excluded.height,
             cup_size = excluded.cup_size,
             birthdate = excluded.birthdate,
             bio = excluded.bio,
             images = excluded.images,
             sort_order = excluded.sort_order,
             updated_at = excluded.updated_at"#,
        params![
            actor.id,
            actor.name,
            serde_json::to_string(&actor.stage_names).unwrap_or_else(|_| "[]".into()),
            actor.height,
            actor.cup_size,
            actor.birthdate,
            actor.bio,
            serde_json::to_string(&actor.images).unwrap_or_else(|_| "[]".into()),
            actor.sort_order,
            actor.created_at,
            actor.updated_at,
        ],
    )
    .map_err(|e| format!("保存演员失败: {e}"))?;
    Ok(())
}

pub fn delete_actor(actor_id: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM video_actors WHERE actor_id = ?1", params![actor_id])
        .map_err(|e| format!("清理关联失败: {e}"))?;
    tx.execute("DELETE FROM actors WHERE id = ?1", params![actor_id])
        .map_err(|e| format!("删除演员失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  标签组 / 标签
// ══════════════════════════════════════════════════════════

pub fn list_tag_groups() -> Result<Vec<TagGroup>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM tag_groups ORDER BY sort_order, created_at")
        .map_err(|e| format!("查询标签组失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TagGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                sort_order: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("读取标签组失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("解析标签组失败: {e}"))?);
    }
    Ok(out)
}

pub fn upsert_tag_group(group: &TagGroup) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        r#"INSERT INTO tag_groups (id, name, sort_order, created_at)
           VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             sort_order = excluded.sort_order"#,
        params![group.id, group.name, group.sort_order, group.created_at],
    )
    .map_err(|e| format!("保存标签组失败: {e}"))?;
    Ok(())
}

pub fn delete_tag_group(group_id: &str) -> Result<(), String> {
    let conn = lock()?;
    conn.execute("DELETE FROM tag_groups WHERE id = ?1", params![group_id])
        .map_err(|e| format!("删除标签组失败: {e}"))?;
    Ok(())
}

pub fn list_tags() -> Result<Vec<Tag>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM tags ORDER BY sort_order, created_at")
        .map_err(|e| format!("查询标签失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                group_id: row.get(3)?,
                sort_order: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("读取标签失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("解析标签失败: {e}"))?);
    }
    Ok(out)
}

pub fn upsert_tag(tag: &Tag) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        r#"INSERT INTO tags (id, name, color, group_id, sort_order, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)
           ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             color = excluded.color,
             group_id = excluded.group_id,
             sort_order = excluded.sort_order"#,
        params![
            tag.id,
            tag.name,
            tag.color,
            tag.group_id,
            tag.sort_order,
            tag.created_at,
        ],
    )
    .map_err(|e| format!("保存标签失败: {e}"))?;
    Ok(())
}

pub fn delete_tag(tag_id: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM video_tags WHERE tag_id = ?1", params![tag_id])
        .map_err(|e| format!("清理关联失败: {e}"))?;
    tx.execute("DELETE FROM tags WHERE id = ?1", params![tag_id])
        .map_err(|e| format!("删除标签失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}
