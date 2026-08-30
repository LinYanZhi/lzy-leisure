//! 漫画书架数据模型与操作（合并进休闲时光单库）
use crate::db::{lock, natural_key};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

// ══════════════════════════════════════════════════════════
//  数据模型
// ══════════════════════════════════════════════════════════

/// 漫画记录（与前端共享的数据结构）
#[derive(Debug, Clone, Serialize)]
pub struct Comic {
    pub id: String,
    pub title: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String, // folder | archive | series
    pub format: String,
    pub page_count: i64,
    pub size: i64,
    pub root_dir: String,
    pub series_id: String,
    pub cover_path: String,
    pub reading_progress: i64,
    pub chapter_count: i64,
    pub sort_order: i64,
    /// 集合父级（role=collection 目录的卡片），不作为书架单元展示
    pub is_container: bool,
    pub updated_at: String,
}

impl Comic {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        title: String,
        path: String,
        kind: String,
        format: String,
        page_count: i64,
        size: i64,
        root_dir: String,
        series_id: String,
        cover_path: String,
        chapter_count: i64,
    ) -> Self {
        Self {
            id,
            title,
            path,
            kind,
            format,
            page_count,
            size,
            root_dir,
            series_id,
            cover_path,
            reading_progress: 0,
            chapter_count,
            sort_order: 0,
            is_container: false,
            updated_at: String::new(),
        }
    }
}

/// 根目录记录
#[derive(Debug, Clone, Serialize)]
pub struct RootDir {
    pub path: String,
    pub name: String,
    pub comic_count: i64,
    pub last_scan: String,
}

/// 封面 BLOB
#[derive(Debug, Clone)]
pub struct CoverData {
    pub data: Vec<u8>,
    pub mime: String,
    pub width: i64,
    pub height: i64,
}

fn row_to_comic(row: &rusqlite::Row) -> rusqlite::Result<Comic> {
    Ok(Comic {
        id: row.get("id")?,
        title: row.get("title")?,
        path: row.get("path")?,
        kind: row.get("type")?,
        format: row.get("format")?,
        page_count: row.get("page_count")?,
        size: row.get("size")?,
        root_dir: row.get("root_dir")?,
        series_id: row.get("series_id")?,
        cover_path: row.get("cover_path")?,
        reading_progress: row.get("reading_progress")?,
        chapter_count: row.get("chapter_count")?,
        sort_order: row.get("sort_order")?,
        is_container: row.get("is_container")?,
        updated_at: row.get("updated_at").unwrap_or_default(),
    })
}

// ══════════════════════════════════════════════════════════
//  根目录管理
// ══════════════════════════════════════════════════════════

pub fn add_root_dir(path: &str) -> Result<(), String> {
    let name = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let conn = lock()?;
    conn.execute(
        "INSERT INTO root_dirs (path, name) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET name = excluded.name",
        params![path, name],
    )
    .map_err(|e| format!("添加根目录失败: {e}"))?;
    Ok(())
}

/// 移除根目录（记录 + 可再生的应用库缓存；**保留**目录外部的封面数据）。
///
/// 封面 BLOB / 手动封面参数 / 页面索引 / 页面预览均已迁往各漫画本体目录的
/// 外部封面库（.leisure-covers.db），不随移除删除——重新添加同一目录后
/// 漫画 id 稳定（路径 hash），封面与手动设置直接复用。
pub fn remove_root_dir(path: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM comics WHERE root_dir = ?1", params![path])
        .map_err(|e| format!("清理漫画失败: {e}"))?;
    tx.execute("DELETE FROM root_dirs WHERE path = ?1", params![path])
        .map_err(|e| format!("删除根目录失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

pub fn get_root_dirs() -> Result<Vec<RootDir>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT r.path AS path, r.name AS name,
                    (SELECT COUNT(*) FROM comics c WHERE c.root_dir = r.path AND c.is_container = 0
                       AND (c.series_id = '' OR c.series_id IN (SELECT id FROM comics WHERE is_container = 1))) AS comic_count,
                    COALESCE((SELECT MAX(updated_at) FROM comics c WHERE c.root_dir = r.path), r.created_at) AS last_scan
             FROM root_dirs r ORDER BY r.sort_order, r.created_at",
        )
        .map_err(|e| format!("查询根目录失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RootDir {
                path: row.get("path")?,
                name: row.get("name")?,
                comic_count: row.get("comic_count")?,
                last_scan: row.get("last_scan")?,
            })
        })
        .map_err(|e| format!("查询根目录失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取根目录失败: {e}"))
}

// ══════════════════════════════════════════════════════════
//  漫画 CRUD
// ══════════════════════════════════════════════════════════

/// 替换某个根目录的全部漫画记录。
/// 使用 UPSERT 保留阅读进度与封面缓存等用户数据，并删除磁盘上已不存在的记录。
pub fn replace_root_comics(root_dir: &str, comics: &[Comic]) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    for comic in comics {
        tx.execute(
            r#"INSERT INTO comics (id, title, path, type, format, page_count, size, root_dir, series_id, cover_path, chapter_count, sort_order, is_container, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
               ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 path = excluded.path,
                 type = excluded.type,
                 format = excluded.format,
                 page_count = excluded.page_count,
                 size = excluded.size,
                 root_dir = excluded.root_dir,
                 series_id = excluded.series_id,
                 cover_path = excluded.cover_path,
                 chapter_count = excluded.chapter_count,
                 is_container = excluded.is_container,
                 updated_at = datetime('now')"#,
            params![
                comic.id,
                comic.title,
                comic.path,
                comic.kind,
                comic.format,
                comic.page_count,
                comic.size,
                root_dir,
                comic.series_id,
                comic.cover_path,
                comic.chapter_count,
                comic.sort_order,
                comic.is_container,
            ],
        )
        .map_err(|e| format!("写入漫画失败: {e}"))?;
    }
    // 删除磁盘上已不存在的记录（保持数据一致性）
    let ids: Vec<String> = comics.iter().map(|c| c.id.clone()).collect();
    if ids.is_empty() {
        tx.execute("DELETE FROM comics WHERE root_dir = ?1", params![root_dir])
            .map_err(|e| format!("清理旧记录失败: {e}"))?;
    } else {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM comics WHERE root_dir = ?1 AND id NOT IN ({placeholders})");
        tx.execute(
            &sql,
            rusqlite::params_from_iter(std::iter::once(root_dir.to_string()).chain(ids)),
        )
        .map_err(|e| format!("清理旧记录失败: {e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 单漫画重扫替换：仅更新该漫画（及其章节），保留同一 root 下其他漫画。
/// upsert 不覆盖 reading_progress / sort_order / completed，保证重扫后进度与排序不丢。
pub fn replace_comic(root_dir: &str, comic_id: &str, comics: &[Comic]) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    for comic in comics {
        tx.execute(
            r#"INSERT INTO comics (id, title, path, type, format, page_count, size, root_dir, series_id, cover_path, chapter_count, sort_order, is_container, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
               ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 path = excluded.path,
                 type = excluded.type,
                 format = excluded.format,
                 page_count = excluded.page_count,
                 size = excluded.size,
                 root_dir = excluded.root_dir,
                 series_id = excluded.series_id,
                 cover_path = excluded.cover_path,
                 chapter_count = excluded.chapter_count,
                 is_container = excluded.is_container,
                 updated_at = datetime('now')"#,
            params![
                comic.id,
                comic.title,
                comic.path,
                comic.kind,
                comic.format,
                comic.page_count,
                comic.size,
                root_dir,
                comic.series_id,
                comic.cover_path,
                comic.chapter_count,
                comic.sort_order,
                comic.is_container,
            ],
        )
        .map_err(|e| format!("写入漫画失败: {e}"))?;
    }
    // 清理该漫画已不存在的记录（自身 id 或其章节），保留同根其他漫画
    let ids: Vec<String> = comics.iter().map(|c| c.id.clone()).collect();
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "DELETE FROM comics WHERE (id = ?1 OR series_id = ?1) AND id NOT IN ({placeholders})"
    );
    tx.execute(
        &sql,
        rusqlite::params_from_iter(std::iter::once(comic_id.to_string()).chain(ids)),
    )
    .map_err(|e| format!("清理旧记录失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 加载全部漫画本体（书架展示单元）：
/// 顶级系列/单本 + 集合目录中抽离出的所有子漫画（嵌套任意层），
/// 不含集合父级卡片（is_container=1）与系列章节（series_id 指向普通系列）。
/// 排序为全局 sort_order（跨根目录，与 index.json 无关）。
pub fn load_top_level_comics() -> Result<Vec<Comic>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT * FROM comics \
             WHERE is_container = 0 \
               AND (series_id = '' OR series_id IN (SELECT id FROM comics WHERE is_container = 1))",
        )
        .map_err(|e| format!("查询漫画失败: {e}"))?;
    let rows = stmt
        .query_map([], row_to_comic)
        .map_err(|e| format!("查询漫画失败: {e}"))?;
    let mut comics = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取漫画失败: {e}"))?;
    // 全局自然排序：sort_order 优先（0 视为未排序排最后），再按标题。
    // 不再按 root_dir 分组——拖拽排序可跨根目录生效（排序存 comics.sort_order）。
    comics.sort_by_key(|c| (c.sort_order == 0, c.sort_order, natural_key(&c.title)));
    Ok(comics)
}

/// 加载全部漫画记录（全表，供封面库归属计算等使用）
pub fn load_all_comics() -> Result<Vec<Comic>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM comics")
        .map_err(|e| format!("查询漫画失败: {e}"))?;
    let rows = stmt
        .query_map([], row_to_comic)
        .map_err(|e| format!("查询漫画失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取漫画失败: {e}"))
}

/// 加载某系列的章节（标题覆盖 index.json）
pub fn load_chapters(series_id: &str) -> Result<Vec<Comic>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM comics WHERE series_id = ?1 ORDER BY title")
        .map_err(|e| format!("查询章节失败: {e}"))?;
    let mut chapters = stmt
        .query_map(params![series_id], row_to_comic)
        .map_err(|e| format!("查询章节失败: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取章节失败: {e}"))?;

    // 用系列目录的 index.json 覆盖标题与排序
    // （容器结构下系列位于 root_dir/系列path，读 root_dir 会拿到容器索引）
    let root_dir = chapters
        .first()
        .map(|c| c.root_dir.clone())
        .unwrap_or_default();
    let series_dir = if root_dir.is_empty() {
        String::new()
    } else {
        let series_path: String = conn
            .query_row(
                "SELECT path FROM comics WHERE id = ?1",
                params![series_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| ".".to_string());
        Path::new(&root_dir).join(&series_path).to_string_lossy().to_string()
    };
    if let Some((title_map, order_map)) = crate::scanner::load_index_json(&series_dir) {
        for ch in &mut chapters {
            if let Some(dir_name) = Path::new(&ch.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
            {
                if let Some(t) = title_map.get(&dir_name) {
                    ch.title = t.clone();
                }
            }
        }
        chapters.sort_by_key(|ch| {
            let dir_name = Path::new(&ch.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            (
                order_map.get(&dir_name).copied().unwrap_or(i64::MAX),
                natural_key(&ch.title),
                ch.title.clone(),
            )
        });
    }
    Ok(chapters)
}

/// 系列续读聚合：返回各章节进度，并推荐最佳续读章节
#[derive(Debug, Clone, Serialize)]
pub struct ChapterProgress {
    pub id: String,
    pub title: String,
    pub page: i64,
    pub total_pages: i64,
    pub pct: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesProgress {
    pub series_id: String,
    pub chapters: Vec<ChapterProgress>,
    /// 推荐继续阅读的章节 id（无则空）
    pub continue_chapter_id: String,
    /// 是否全部读完
    pub all_completed: bool,
}

fn compute_series_progress(series_id: &str, chapters: &[Comic]) -> SeriesProgress {
    let mut list: Vec<ChapterProgress> = Vec::with_capacity(chapters.len());
    let mut best_id = String::new();
    let mut best_pct = 0;
    let mut all_completed = true;

    for ch in chapters {
        let total = ch.page_count.max(1);
        let pct = ((ch.reading_progress as f64 / total as f64) * 100.0).round() as i64;
        list.push(ChapterProgress {
            id: ch.id.clone(),
            title: ch.title.clone(),
            page: ch.reading_progress,
            total_pages: ch.page_count,
            pct,
        });
        if pct < 90 {
            all_completed = false;
        }
        // 仅「读了一部分但未读完」的章节参与续读推荐，未读章节不推荐
        if pct > 0 && pct < 90 && pct > best_pct {
            best_pct = pct;
            best_id = ch.id.clone();
        }
    }
    // 全部读完时推荐最后一章
    if all_completed && !chapters.is_empty() {
        best_id = chapters.last().map(|c| c.id.clone()).unwrap_or_default();
    }

    SeriesProgress {
        series_id: series_id.to_string(),
        chapters: list,
        continue_chapter_id: best_id,
        all_completed,
    }
}

pub fn get_series_progress(series_id: &str) -> Result<SeriesProgress, String> {
    let chapters = load_chapters(series_id)?;
    Ok(compute_series_progress(series_id, &chapters))
}

/// 一次获取全部系列的续读聚合（书架渲染用）。
/// 遍历所有非容器系列（含集合中抽离的子系列），进度 key 为系列本体 id。
pub fn get_all_series_progress() -> Result<HashMap<String, SeriesProgress>, String> {
    let all_series: Vec<Comic> = {
        // 块作用域内持锁，结束后立即释放——load_chapters 内部会再次 lock()，
        // std::sync::Mutex 不可重入，持锁循环调用会死锁
        let conn = lock()?;
        let mut stmt = conn
            .prepare("SELECT * FROM comics WHERE type = 'series' AND is_container = 0")
            .map_err(|e| format!("查询系列失败: {e}"))?;
        let rows = stmt
            .query_map([], row_to_comic)
            .map_err(|e| format!("查询系列失败: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("读取系列失败: {e}"))?
    };

    let mut map: HashMap<String, SeriesProgress> = HashMap::new();
    for s in &all_series {
        let chapters = load_chapters(&s.id)?;
        if !chapters.is_empty() {
            // 标题取 index.json 覆盖后的显示名
            map.insert(s.id.clone(), compute_series_progress(&s.id, &chapters));
        }
    }
    Ok(map)
}

pub fn get_comic(comic_id: &str) -> Result<Option<Comic>, String> {
    let conn = lock()?;
    conn.query_row(
        "SELECT * FROM comics WHERE id = ?1",
        params![comic_id],
        row_to_comic,
    )
    .optional()
    .map_err(|e| format!("查询漫画失败: {e}"))
}

/// 更新漫画的允许字段（标题等）
pub fn update_comic(comic_id: &str, title: &str) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "UPDATE comics SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![title, comic_id],
    )
    .map_err(|e| format!("更新漫画失败: {e}"))?;
    Ok(())
}

/// 删除漫画（含其章节的级联清理）
/// 注意：封面已移至各漫画本体目录的外部封面库（.leisure-covers.db），
/// 删除记录不触碰外部库——移除/重导后封面直接复用。
pub fn delete_comic(comic_id: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM comics WHERE series_id = ?1", params![comic_id])
        .map_err(|e| format!("删除章节失败: {e}"))?;
    tx.execute("DELETE FROM comics WHERE id = ?1", params![comic_id])
        .map_err(|e| format!("删除漫画失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 批量更新排序（拖拽排序）
pub fn batch_set_sort_order(orders: &[(i64, String)]) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    for (order, id) in orders {
        tx.execute("UPDATE comics SET sort_order = ?1 WHERE id = ?2", params![order, id])
            .map_err(|e| format!("更新排序失败: {e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  阅读进度
// ══════════════════════════════════════════════════════════

pub fn get_reading_progress(comic_id: &str) -> Result<i64, String> {
    let conn = lock()?;
    let p = conn
        .query_row(
            "SELECT reading_progress FROM comics WHERE id = ?1",
            params![comic_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("查询进度失败: {e}"))?;
    Ok(p.unwrap_or(0))
}

pub fn set_reading_progress(comic_id: &str, page: i64) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "UPDATE comics SET reading_progress = ?1 WHERE id = ?2",
        params![page, comic_id],
    )
    .map_err(|e| format!("保存进度失败: {e}"))?;
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  完结标志（DB 缓存，权威在 index.json）
// ══════════════════════════════════════════════════════════

/// 读取完结缓存（NULL = 未缓存，调用方回退读 index.json 并回填）
pub fn get_completed(comic_id: &str) -> Result<Option<bool>, String> {
    let conn = lock()?;
    conn.query_row(
        "SELECT completed FROM comics WHERE id = ?1",
        params![comic_id],
        |r| r.get::<_, Option<i64>>(0).map(|v| v.map(|x| x != 0)),
    )
    .optional()
    .map_err(|e| format!("查询完结失败: {e}"))
    .map(|v| v.flatten())
}

/// 写入完结缓存
pub fn set_completed(comic_id: &str, completed: bool) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "UPDATE comics SET completed = ?1 WHERE id = ?2",
        params![completed as i64, comic_id],
    )
    .map_err(|e| format!("保存完结失败: {e}"))?;
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  根目录扫描辅助
// ══════════════════════════════════════════════════════════

/// 读取某根目录下全部漫画的 cover_path（扫描前后对比用）
pub fn load_root_cover_paths(root_dir: &str) -> Result<HashMap<String, String>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT id, cover_path FROM comics WHERE root_dir = ?1")
        .map_err(|e| format!("查询封面路径失败: {e}"))?;
    let rows = stmt
        .query_map(params![root_dir], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("查询封面路径失败: {e}"))?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("读取封面路径失败: {e}"))
}

/// 读取某根目录下全部漫画（扫描增量页数缓存用，避免重复解析 PDF/压缩包）
pub fn load_root_comics(root_dir: &str) -> Result<Vec<Comic>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM comics WHERE root_dir = ?1")
        .map_err(|e| format!("查询漫画失败: {e}"))?;
    let rows = stmt
        .query_map(params![root_dir], row_to_comic)
        .map_err(|e| format!("查询漫画失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取漫画失败: {e}"))
}
