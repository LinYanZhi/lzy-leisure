//! 小说书架数据模型与操作（单本 EPUB = 一部书）
use crate::db::{lock, now};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

// ══════════════════════════════════════════════════════════
//  数据模型
// ══════════════════════════════════════════════════════════

/// 小说记录（与前端共享的数据结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Novel {
    pub id: String,
    pub title: String,
    pub author: String,
    pub path: String,
    pub cover_path: String,
    /// 阅读进度：章节内滚动位置（字符偏移），0 表示未读
    pub reading_pos: i64,
    /// 当前章节（spine 索引，0 起）
    pub chapter: String,
    /// 归属导入源；单文件导入为空串
    pub root_dir: String,
    pub chapter_count: i64,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    /// 章节列表缓存（解析自 EPUB，重扫刷新）
    pub chapters: Vec<NovelChapter>,
}

/// 章节（id 即 spine 索引）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NovelChapter {
    pub id: i64,
    pub title: String,
    pub href: String,
}

/// 根目录记录
#[derive(Debug, Clone, Serialize)]
pub struct NovelRootDir {
    pub path: String,
    pub name: String,
    pub novel_count: i64,
    pub last_scan: String,
}

fn row_to_novel(row: &rusqlite::Row) -> rusqlite::Result<Novel> {
    let chapters_json: String = row.get("chapters_json")?;
    let chapters = serde_json::from_str(&chapters_json).unwrap_or_default();
    Ok(Novel {
        id: row.get("id")?,
        title: row.get("title")?,
        author: row.get("author")?,
        path: row.get("path")?,
        cover_path: row.get("cover_path")?,
        reading_pos: row.get("reading_pos")?,
        chapter: row.get("chapter")?,
        root_dir: row.get("root_dir")?,
        chapter_count: row.get("chapter_count")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        chapters,
    })
}

// ══════════════════════════════════════════════════════════
//  根目录
// ══════════════════════════════════════════════════════════

pub fn add_novel_root(path: &str, name: &str) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "INSERT INTO novel_roots (path, name, created_at) VALUES (?1, ?2, datetime('now', 'localtime'))
         ON CONFLICT(path) DO UPDATE SET name = excluded.name",
        params![path, name],
    )
    .map_err(|e| format!("保存目录失败: {e}"))?;
    Ok(())
}

/// 移除导入路径：删除路径记录及其下全部书（不碰磁盘文件）
pub fn remove_novel_root(path: &str) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM novels WHERE root_dir = ?1", params![path])
        .map_err(|e| format!("清理书籍失败: {e}"))?;
    tx.execute("DELETE FROM novel_roots WHERE path = ?1", params![path])
        .map_err(|e| format!("删除路径记录失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

pub fn get_novel_roots() -> Result<Vec<NovelRootDir>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT r.path AS path, r.name AS name,
                    (SELECT COUNT(*) FROM novels n WHERE n.root_dir = r.path) AS novel_count,
                    COALESCE((SELECT MAX(updated_at) FROM novels n WHERE n.root_dir = r.path), r.created_at) AS last_scan
             FROM novel_roots r ORDER BY r.sort_order, r.created_at",
        )
        .map_err(|e| format!("查询书籍目录失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(NovelRootDir {
                path: row.get("path")?,
                name: row.get("name")?,
                novel_count: row.get("novel_count")?,
                last_scan: row.get("last_scan")?,
            })
        })
        .map_err(|e| format!("查询书籍目录失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取书籍目录失败: {e}"))
}

// ══════════════════════════════════════════════════════════
//  书
// ══════════════════════════════════════════════════════════

pub fn list_novels() -> Result<Vec<Novel>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM novels ORDER BY sort_order, title")
        .map_err(|e| format!("查询书籍失败: {e}"))?;
    let rows = stmt
        .query_map([], row_to_novel)
        .map_err(|e| format!("查询书籍失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取书籍失败: {e}"))
}

pub fn get_novel(novel_id: &str) -> Result<Option<Novel>, String> {
    let conn = lock()?;
    conn.query_row("SELECT * FROM novels WHERE id = ?1", params![novel_id], row_to_novel)
        .optional()
        .map_err(|e| format!("查询书籍失败: {e}"))
}

pub fn get_novel_by_path(path: &str) -> Result<Option<Novel>, String> {
    let conn = lock()?;
    conn.query_row("SELECT * FROM novels WHERE path = ?1", params![path], row_to_novel)
        .optional()
        .map_err(|e| format!("查询书籍失败: {e}"))
}

/// 保存书（按 path 幂等）：扫描/导入时登记元信息，不覆盖阅读进度
pub fn upsert_novel(novel: &Novel) -> Result<(), String> {
    let chapters_json = serde_json::to_string(&novel.chapters).unwrap_or_else(|_| "[]".to_string());
    let conn = lock()?;
    conn.execute(
        r#"INSERT INTO novels (id, title, author, path, cover_path, reading_pos, chapter, root_dir, chapter_count, chapters_json, sort_order, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, 0, '', ?6, ?7, ?8, ?9, datetime('now', 'localtime'), datetime('now', 'localtime'))
           ON CONFLICT(path) DO UPDATE SET
             id = excluded.id,
             title = excluded.title,
             author = excluded.author,
             root_dir = excluded.root_dir,
             chapter_count = excluded.chapter_count,
             chapters_json = excluded.chapters_json,
             updated_at = excluded.updated_at"#,
        params![
            novel.id,
            novel.title,
            novel.author,
            novel.path,
            novel.cover_path,
            novel.root_dir,
            novel.chapter_count,
            chapters_json,
            novel.sort_order,
        ],
    )
    .map_err(|e| format!("保存书籍失败: {e}"))?;
    Ok(())
}

pub fn delete_novel(novel_id: &str) -> Result<(), String> {
    let conn = lock()?;
    conn.execute("DELETE FROM novels WHERE id = ?1", params![novel_id])
        .map_err(|e| format!("删除书籍失败: {e}"))?;
    Ok(())
}

/// 删除某个导入路径下磁盘已不存在的书（重扫后保持一致性），返回删除数量。
/// 注意：不能持锁调用 delete_novel（Mutex 不可重入），改为在本事务内直接删除。
pub fn delete_novels_not_in(root_dir: &str, keep_paths: &[String]) -> Result<usize, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT id, path FROM novels WHERE root_dir = ?1")
        .map_err(|e| format!("查询书籍失败: {e}"))?;
    let rows = stmt
        .query_map(params![root_dir], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("查询书籍失败: {e}"))?;
    let mut gone: Vec<String> = Vec::new();
    for r in rows.flatten() {
        if !keep_paths.contains(&r.1) {
            gone.push(r.0);
        }
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    let mut removed = 0usize;
    for id in &gone {
        tx.execute("DELETE FROM novels WHERE id = ?1", params![id])
            .map_err(|e| format!("删除书籍失败: {e}"))?;
        removed += 1;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(removed)
}

/// 更新阅读进度（chapter 为 spine 索引字符串，pos 为章节内字符偏移）
pub fn set_novel_progress(novel_id: &str, chapter: &str, pos: i64) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = now();
    let conn = lock()?;
    conn.execute(
        "UPDATE novels SET chapter = ?1, reading_pos = ?2, updated_at = ?3 WHERE id = ?4",
        params![chapter, pos, ts, novel_id],
    )
    .map_err(|e| format!("保存阅读进度失败: {e}"))?;
    Ok(())
}

/// 更新书元信息（重扫/改名后用）
pub fn update_novel_meta(
    novel_id: &str,
    title: &str,
    author: &str,
    chapter_count: i64,
    chapters_json: &str,
) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = now();
    let conn = lock()?;
    conn.execute(
        "UPDATE novels SET title = ?1, author = ?2, chapter_count = ?3, chapters_json = ?4, updated_at = ?5 WHERE id = ?6",
        params![title, author, chapter_count, chapters_json, ts, novel_id],
    )
    .map_err(|e| format!("更新书籍失败: {e}"))?;
    Ok(())
}

/// 记录封面缓存路径（相对应用数据目录）
pub fn set_novel_cover(novel_id: &str, cover_path: &str) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = now();
    let conn = lock()?;
    conn.execute(
        "UPDATE novels SET cover_path = ?1, updated_at = ?2 WHERE id = ?3",
        params![cover_path, ts, novel_id],
    )
    .map_err(|e| format!("保存封面失败: {e}"))?;
    Ok(())
}

/// 改名（只改库记录，不碰文件）
pub fn rename_novel(novel_id: &str, title: &str) -> Result<(), String> {
    // now() 内部会获取数据库锁，必须先于 lock() 调用，避免同一 Mutex 非重入死锁
    let ts = now();
    let conn = lock()?;
    conn.execute(
        "UPDATE novels SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, ts, novel_id],
    )
    .map_err(|e| format!("改名失败: {e}"))?;
    Ok(())
}
