//! 视频/演员/标签数据模型与操作（合并进休闲时光单库）
//!
//! 按职责拆分为子模块（外部 crate::db::videos::* 路径不变）：
//!   - `videos_actors.rs` 演员与标签组/标签
//!   - `videos_series.rs` 剧集（Series）聚合
pub(crate) mod videos_actors;
pub(crate) mod videos_series;

// 重导出：保持 crate::db::videos::* 外部路径不变
pub use videos_actors::{
    delete_actor, delete_tag, delete_tag_group, get_actor, list_actors, list_tag_groups, list_tags,
    upsert_actor, upsert_tag, upsert_tag_group,
};
pub use videos_series::{
    create_series, delete_series, get_series, get_series_videos, list_series, set_videos_series,
    update_series, update_video_progress, Series,
};

use crate::db::lock;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::path::Path;

// ══════════════════════════════════════════════════════════
//  数据模型
// ══════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub path: String,
    pub description: String,
    pub license_plate: String,
    pub cover: String,
    pub duration: String,
    pub file_size: String,
    pub file_type: String,
    pub fps: Option<f64>,
    pub frame_width: Option<i64>,
    pub frame_height: Option<i64>,
    pub sort_order: i64,
    pub actors: Vec<String>,
    pub tags: Vec<String>,
    /// 种类（电影/短视频/动漫/竖屏，可多选），存 JSON 数组
    pub kinds: Vec<String>,
    /// 同目录完全同名（仅扩展名不同）的字幕文件路径，扫描时登记；空表示无字幕
    pub subtitle_path: String,
    /// 归属的导入路径（源）；扫描时填充，用于按源过滤与整源移除
    pub root_dir: String,
    /// 所属剧集（series.id）；空串表示不属于任何剧集
    pub series_id: String,
    /// 播放进度（秒），用于续播；0 表示未观看
    pub progress: f64,
    /// 年份（如 "2020"）；文件名解析或手动填写；空串表示未知
    pub year: String,
    /// 评分（0-10）；0 表示未评分
    pub rating: f64,
    /// 集数标识（如 "第01话" / "S01E05"）；文件名解析填充；空串表示无集数
    pub episode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Actor {
    pub id: String,
    pub name: String,
    pub stage_names: Vec<String>,
    pub height: String,
    pub cup_size: String,
    pub birthdate: String,
    pub bio: String,
    pub images: Vec<String>,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagGroup {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub group_id: String,
    pub sort_order: i64,
    pub created_at: String,
}
// ══════════════════════════════════════════════════════════
//  视频
// ══════════════════════════════════════════════════════════

pub(crate) fn row_to_video(row: &rusqlite::Row) -> rusqlite::Result<Video> {
    Ok(Video {
        id: row.get(0)?,
        title: row.get(1)?,
        path: row.get(2)?,
        description: row.get(3)?,
        license_plate: row.get(4)?,
        cover: row.get(5)?,
        duration: row.get(6)?,
        file_size: row.get(7)?,
        file_type: row.get(8)?,
        fps: row.get(9)?,
        frame_width: row.get(10)?,
        frame_height: row.get(11)?,
        sort_order: row.get(12)?,
        actors: Vec::new(),
        tags: Vec::new(),
        kinds: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
        subtitle_path: row.get(16)?,
        root_dir: row.get(17)?,
        series_id: row.get(18)?,
        progress: row.get(19)?,
        year: row.get(20)?,
        rating: row.get(21)?,
        episode: row.get(22)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

/// 查询筛选参数
#[derive(Default, Clone)]
pub struct VideoFilter {
    pub title: Option<String>,
    pub tag_ids: Vec<String>,
    pub match_all: bool,
    pub actor_ids: Vec<String>,
    pub vertical_only: bool,
    /// 种类过滤（任一匹配即命中）
    pub kinds: Vec<String>,
    /// 按导入路径（源）过滤；None 表示全部
    pub root_dir: Option<String>,
    /// 按剧集过滤；None 表示全部（"unassigned" 表示未归入任何剧集）
    pub series_id: Option<String>,
}

/// 列表查询（含演员/标签聚合），按 sort_order 排序
pub fn list_videos(filter: &VideoFilter) -> Result<Vec<Video>, String> {
    let conn = lock()?;
    let mut sql = String::from("SELECT * FROM videos");
    let mut conds: Vec<String> = Vec::new();
    let mut vals: Vec<String> = Vec::new();

    if let Some(t) = &filter.title {
        if !t.is_empty() {
            conds.push("title LIKE ?".into());
            vals.push(format!("%{t}%"));
        }
    }
    if filter.vertical_only {
        conds.push("frame_width IS NOT NULL AND frame_height IS NOT NULL AND frame_height > frame_width".into());
    }
    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }
    sql.push_str(" ORDER BY sort_order, created_at");

    let mut stmt = conn.prepare(&sql).map_err(|e| format!("查询视频失败: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(vals.iter()), |row| row_to_video(row))
        .map_err(|e| format!("读取视频失败: {e}"))?;
    let mut videos = Vec::new();
    for r in rows {
        videos.push(r.map_err(|e| format!("解析视频失败: {e}"))?);
    }

    // 标签/演员/种类/导入路径/剧集筛选（内存过滤，数据量小）
    if !filter.tag_ids.is_empty()
        || !filter.actor_ids.is_empty()
        || !filter.kinds.is_empty()
        || filter.root_dir.is_some()
        || filter.series_id.is_some()
    {
        videos.retain(|v| {
            let mut ok = true;
            if !filter.tag_ids.is_empty() {
                let hit = if filter.match_all {
                    filter.tag_ids.iter().all(|t| v.tags.contains(t))
                } else {
                    filter.tag_ids.iter().any(|t| v.tags.contains(t))
                };
                ok &= hit;
            }
            if !filter.actor_ids.is_empty() {
                ok &= filter.actor_ids.iter().any(|a| v.actors.contains(a));
            }
            if !filter.kinds.is_empty() {
                ok &= filter.kinds.iter().any(|k| v.kinds.contains(k));
            }
            if let Some(root) = &filter.root_dir {
                ok &= v.root_dir == *root;
            }
            if let Some(sid) = &filter.series_id {
                ok &= if sid == "unassigned" {
                    v.series_id.is_empty()
                } else {
                    v.series_id == *sid
                };
            }
            ok
        });
    }

    Ok(videos)
}

// ══════════════════════════════════════════════════════════
//  导入路径（源）管理
// ══════════════════════════════════════════════════════════

/// 视频导入路径记录（源管理弹窗展示单元）
#[derive(Debug, Clone, Serialize)]
pub struct VideoRootDir {
    pub path: String,
    pub name: String,
    pub video_count: i64,
    pub last_scan: String,
}

pub fn add_video_root(path: &str) -> Result<(), String> {
    let name = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let conn = lock()?;
    conn.execute(
        "INSERT INTO video_roots (path, name) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET name = excluded.name",
        params![path, name],
    )
    .map_err(|e| format!("添加视频目录失败: {e}"))?;
    Ok(())
}

/// 移除导入路径：删除路径记录及其下全部视频（演员/标签关联随外键级联删除）
pub fn remove_video_root(path: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM videos WHERE root_dir = ?1", params![path])
        .map_err(|e| format!("清理视频失败: {e}"))?;
    tx.execute("DELETE FROM video_roots WHERE path = ?1", params![path])
        .map_err(|e| format!("删除路径记录失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

pub fn get_video_roots() -> Result<Vec<VideoRootDir>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT r.path AS path, r.name AS name,
                    (SELECT COUNT(*) FROM videos v WHERE v.root_dir = r.path) AS video_count,
                    COALESCE((SELECT MAX(updated_at) FROM videos v WHERE v.root_dir = r.path), r.created_at) AS last_scan
             FROM video_roots r ORDER BY r.sort_order, r.created_at",
        )
        .map_err(|e| format!("查询视频目录失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(VideoRootDir {
                path: row.get("path")?,
                name: row.get("name")?,
                video_count: row.get("video_count")?,
                last_scan: row.get("last_scan")?,
            })
        })
        .map_err(|e| format!("查询视频目录失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取视频目录失败: {e}"))
}

/// 删除某个导入路径下磁盘已不存在的视频记录（重扫后保持一致性），返回删除数量
pub fn delete_videos_not_in(root_dir: &str, keep_paths: &[String]) -> Result<usize, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT id, path FROM videos WHERE root_dir = ?1")
        .map_err(|e| format!("查询视频失败: {e}"))?;
    let rows = stmt
        .query_map(params![root_dir], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("查询视频失败: {e}"))?;
    let mut gone: Vec<String> = Vec::new();
    for r in rows.flatten() {
        if !keep_paths.contains(&r.1) {
            gone.push(r.0);
        }
    }
    for id in &gone {
        delete_video(id)?;
    }
    Ok(gone.len())
}

pub(crate) fn load_relations(conn: &rusqlite::Connection, video: &mut Video) -> rusqlite::Result<()> {
    let mut sa = conn.prepare("SELECT actor_id FROM video_actors WHERE video_id = ?1")?;
    let rows = sa.query_map(params![video.id], |r| r.get::<_, String>(0))?;
    for r in rows {
        video.actors.push(r?);
    }
    let mut st = conn.prepare("SELECT tag_id FROM video_tags WHERE video_id = ?1")?;
    let rows = st.query_map(params![video.id], |r| r.get::<_, String>(0))?;
    for r in rows {
        video.tags.push(r?);
    }
    Ok(())
}

pub fn get_video(video_id: &str) -> Result<Option<Video>, String> {
    let conn = lock()?;
    let mut video = conn
        .query_row(
            "SELECT * FROM videos WHERE id = ?1",
            params![video_id],
            |row| row_to_video(row),
        )
        .optional()
        .map_err(|e| format!("查询视频失败: {e}"))?;
    if let Some(v) = &mut video {
        load_relations(&conn, v).map_err(|e| format!("读取关联失败: {e}"))?;
    }
    Ok(video)
}

pub fn get_video_by_path(path: &str) -> Result<Option<Video>, String> {
    let conn = lock()?;
    let mut video = conn
        .query_row("SELECT * FROM videos WHERE path = ?1", params![path], |row| {
            row_to_video(row)
        })
        .optional()
        .map_err(|e| format!("查询视频失败: {e}"))?;
    if let Some(v) = &mut video {
        load_relations(&conn, v).map_err(|e| format!("读取关联失败: {e}"))?;
    }
    Ok(video)
}

/// 保存视频（存在则更新元信息，不覆盖 actors/tags/排序）
pub fn upsert_video(video: &Video) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        r#"INSERT INTO videos (id, title, path, description, license_plate, cover, duration, file_size, file_type, fps, frame_width, frame_height, sort_order, created_at, updated_at, kinds, subtitle_path, root_dir, year, episode)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
           ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             path = excluded.path,
             description = excluded.description,
             license_plate = excluded.license_plate,
             cover = excluded.cover,
             duration = excluded.duration,
             file_size = excluded.file_size,
             file_type = excluded.file_type,
             fps = excluded.fps,
             frame_width = excluded.frame_width,
             frame_height = excluded.frame_height,
             kinds = excluded.kinds,
             subtitle_path = excluded.subtitle_path,
             root_dir = excluded.root_dir,
             year = excluded.year,
             episode = excluded.episode,
             updated_at = excluded.updated_at"#,
        params![
            video.id,
            video.title,
            video.path,
            video.description,
            video.license_plate,
            video.cover,
            video.duration,
            video.file_size,
            video.file_type,
            video.fps,
            video.frame_width,
            video.frame_height,
            video.sort_order,
            video.created_at,
            video.updated_at,
            serde_json::to_string(&video.kinds).unwrap_or_else(|_| "[]".into()),
            video.subtitle_path,
            video.root_dir,
            video.year,
            video.episode,
        ],
    )
    .map_err(|e| format!("保存视频失败: {e}"))?;
    Ok(())
}

/// 更新视频的字幕文件路径（扫描时登记，空串表示清除）
pub fn update_video_subtitle(video_id: &str, subtitle_path: &str) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    conn.execute(
        "UPDATE videos SET subtitle_path = ?1, updated_at = ?2 WHERE id = ?3",
        params![subtitle_path, ts, video_id],
    )
    .map_err(|e| format!("更新字幕路径失败: {e}"))?;
    Ok(())
}

/// 更新视频的扫描元信息（字幕路径 + 归属导入路径），重扫已存在视频时调用
pub fn update_video_scan_meta(
    video_id: &str,
    subtitle_path: &str,
    root_dir: &str,
) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    conn.execute(
        "UPDATE videos SET subtitle_path = ?1, root_dir = ?2, updated_at = ?3 WHERE id = ?4",
        params![subtitle_path, root_dir, ts, video_id],
    )
    .map_err(|e| format!("更新视频扫描元信息失败: {e}"))?;
    Ok(())
}

/// 更新视频媒体元数据（前端按需补全：时长/分辨率/大小/格式）。
/// kinds 传 None 时保留原值（避免覆盖用户手动调整）；传 Some 时覆盖。
pub fn update_video_media_meta(
    video_id: &str,
    duration: &str,
    file_size: &str,
    file_type: &str,
    frame_width: Option<i64>,
    frame_height: Option<i64>,
    kinds: Option<&[String]>,
) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    let kinds_json = kinds.map(|k| serde_json::to_string(k).unwrap_or_else(|_| "[]".into()));
    conn.execute(
        "UPDATE videos SET duration = ?1, file_size = ?2, file_type = ?3, frame_width = ?4, frame_height = ?5, kinds = COALESCE(?6, kinds), updated_at = ?7 WHERE id = ?8",
        params![
            duration,
            file_size,
            file_type,
            frame_width,
            frame_height,
            kinds_json,
            ts,
            video_id,
        ],
    )
    .map_err(|e| format!("更新视频媒体元数据失败: {e}"))?;
    Ok(())
}

/// 更新视频的可编辑字段（标题/描述/车牌/年份/评分/种类/演员/标签）
#[allow(clippy::too_many_arguments)]
pub fn update_video_editable(
    video_id: &str,
    title: &str,
    description: &str,
    license_plate: &str,
    year: &str,
    rating: f64,
    kinds: &[String],
    actor_ids: &[String],
    tag_ids: &[String],
) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute(
        "UPDATE videos SET title = ?1, description = ?2, license_plate = ?3, year = ?4, rating = ?5, kinds = ?6, updated_at = ?7 WHERE id = ?8",
        params![
            title,
            description,
            license_plate,
            year,
            rating,
            serde_json::to_string(kinds).unwrap_or_else(|_| "[]".into()),
            ts,
            video_id,
        ],
    )
    .map_err(|e| format!("更新视频失败: {e}"))?;

    tx.execute("DELETE FROM video_actors WHERE video_id = ?1", params![video_id])
        .map_err(|e| format!("清理演员关联失败: {e}"))?;
    for aid in actor_ids {
        tx.execute(
            "INSERT INTO video_actors (video_id, actor_id) VALUES (?1, ?2)",
            params![video_id, aid],
        )
        .map_err(|e| format!("保存演员关联失败: {e}"))?;
    }

    tx.execute("DELETE FROM video_tags WHERE video_id = ?1", params![video_id])
        .map_err(|e| format!("清理标签关联失败: {e}"))?;
    for tid in tag_ids {
        tx.execute(
            "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)",
            params![video_id, tid],
        )
        .map_err(|e| format!("保存标签关联失败: {e}"))?;
    }

    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

pub fn set_video_cover(video_id: &str, cover: &str) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = crate::db::now();
    let conn = lock()?;
    conn.execute(
        "UPDATE videos SET cover = ?1, updated_at = ?2 WHERE id = ?3",
        params![cover, ts, video_id],
    )
    .map_err(|e| format!("保存封面失败: {e}"))?;
    Ok(())
}

pub fn delete_video(video_id: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM video_actors WHERE video_id = ?1", params![video_id])
        .map_err(|e| format!("清理关联失败: {e}"))?;
    tx.execute("DELETE FROM video_tags WHERE video_id = ?1", params![video_id])
        .map_err(|e| format!("清理关联失败: {e}"))?;
    tx.execute("DELETE FROM videos WHERE id = ?1", params![video_id])
        .map_err(|e| format!("删除视频失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 批量更新排序（items: (sort_order, video_id)）
pub fn batch_set_sort_order(orders: &[(i64, String)]) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    for (order, id) in orders {
        tx.execute("UPDATE videos SET sort_order = ?1 WHERE id = ?2", params![order, id])
            .map_err(|e| format!("更新排序失败: {e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}
