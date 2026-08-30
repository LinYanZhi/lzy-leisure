//! 视频库——剧集（Series）聚合层。
//! 原 db/videos.rs 拆分（数据模型与视频见 videos.rs，演员/标签见 videos_actors）。
use super::{load_relations, row_to_video, Video};
use crate::db::lock;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

// ══════════════════════════════════════════════════════════
//  剧集（Series）：多个视频聚合为一部剧/一个合集
// ══════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct Series {
    pub id: String,
    pub title: String,
    pub cover: String,
    pub description: String,
    pub video_count: i64,
    /// 剧集封面候选：取剧集内最早登记的视频 id（前端用其封面图展示剧集卡）
    pub cover_video_id: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list_series() -> Result<Vec<Series>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.title, s.cover, s.description, s.sort_order, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM videos v WHERE v.series_id = s.id) AS video_count,
                    (SELECT v.id FROM videos v WHERE v.series_id = s.id ORDER BY v.created_at LIMIT 1) AS cover_video_id
             FROM series s ORDER BY s.sort_order, s.created_at",
        )
        .map_err(|e| format!("查询剧集失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Series {
                id: row.get(0)?,
                title: row.get(1)?,
                cover: row.get(2)?,
                description: row.get(3)?,
                video_count: row.get(7)?,
                cover_video_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                sort_order: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("查询剧集失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取剧集失败: {e}"))
}

pub fn get_series(series_id: &str) -> Result<Option<Series>, String> {
    let conn = lock()?;
    conn.query_row(
        "SELECT s.id, s.title, s.cover, s.description, s.sort_order, s.created_at, s.updated_at,
                (SELECT COUNT(*) FROM videos v WHERE v.series_id = s.id) AS video_count,
                (SELECT v.id FROM videos v WHERE v.series_id = s.id ORDER BY v.created_at LIMIT 1) AS cover_video_id
         FROM series s WHERE s.id = ?1",
        params![series_id],
        |row| {
            Ok(Series {
                id: row.get(0)?,
                title: row.get(1)?,
                cover: row.get(2)?,
                description: row.get(3)?,
                video_count: row.get(7)?,
                cover_video_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                sort_order: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("查询剧集失败: {e}"))
}

pub fn create_series(title: &str, description: &str) -> Result<Series, String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let id = crate::db::video_id(&format!("series:{title}:{ts}"));
    let conn = lock()?;
    conn.execute(
        "INSERT INTO series (id, title, cover, description, sort_order, created_at, updated_at)
         VALUES (?1, ?2, '', ?3, 0, ?4, ?4)",
        params![id, title, description, ts],
    )
    .map_err(|e| format!("创建剧集失败: {e}"))?;
    Ok(Series {
        id,
        title: title.to_string(),
        cover: String::new(),
        description: description.to_string(),
        video_count: 0,
        cover_video_id: String::new(),
        sort_order: 0,
        created_at: ts.clone(),
        updated_at: ts,
    })
}

pub fn update_series(series_id: &str, title: &str, description: &str) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    conn.execute(
        "UPDATE series SET title = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
        params![title, description, ts, series_id],
    )
    .map_err(|e| format!("更新剧集失败: {e}"))?;
    Ok(())
}

/// 删除剧集：解除其下全部视频的归属（视频本身保留），再删除剧集记录
pub fn delete_series(series_id: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute(
        "UPDATE videos SET series_id = '' WHERE series_id = ?1",
        params![series_id],
    )
    .map_err(|e| format!("解除视频归属失败: {e}"))?;
    tx.execute("DELETE FROM series WHERE id = ?1", params![series_id])
        .map_err(|e| format!("删除剧集失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 批量设置视频归属：video_ids 归入剧集；传入空列表表示仅清除该剧集下已选视频的归属
pub fn set_videos_series(series_id: &str, video_ids: &[String]) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    for id in video_ids {
        tx.execute(
            "UPDATE videos SET series_id = ?1 WHERE id = ?2",
            params![series_id, id],
        )
        .map_err(|e| format!("设置视频归属失败: {e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 从集数标识提取数值（"01" → 1；"第12话" → 12；"S01E05" → 5）；无法识别返回 None
fn episode_num(s: &str) -> Option<i64> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// 读取剧集下全部视频（按集数数值排序，无集数的按标题自然排序兜底）
pub fn get_series_videos(series_id: &str) -> Result<Vec<Video>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM videos WHERE series_id = ?1")
        .map_err(|e| format!("查询剧集视频失败: {e}"))?;
    let rows = stmt
        .query_map(params![series_id], |row| row_to_video(row))
        .map_err(|e| format!("读取剧集视频失败: {e}"))?;
    let mut videos = Vec::new();
    for r in rows {
        videos.push(r.map_err(|e| format!("解析剧集视频失败: {e}"))?);
    }
    for v in &mut videos {
        load_relations(&conn, v).map_err(|e| format!("读取关联失败: {e}"))?;
    }
    videos.sort_by(|a, b| match (episode_num(&a.episode), episode_num(&b.episode)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => crate::db::natural_key(&a.title).cmp(&crate::db::natural_key(&b.title)),
    });
    Ok(videos)
}

/// 保存播放进度（秒）；0 表示从头开始/看完重置
pub fn update_video_progress(video_id: &str, seconds: f64) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    conn.execute(
        "UPDATE videos SET progress = ?1, updated_at = ?2 WHERE id = ?3",
        params![seconds, ts, video_id],
    )
    .map_err(|e| format!("保存播放进度失败: {e}"))?;
    Ok(())
}
