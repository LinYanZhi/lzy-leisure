//! 故事会数据模型与操作（单一路径：根目录下每个子目录 = 一年，内含 PDF 期数）
//!
//! 与漫画/视频不同，故事会只维护一个导入路径（meta 表 storyclub_root），
//! 期数记录存 storyclub_issues 表；进度/页数等软数据存库，PDF 本体仍在磁盘。
use crate::db::{lock, natural_key};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// 故事会导入路径在 meta 表中的 key（单一路径，重新设置即替换）
pub const ROOT_KEY: &str = "storyclub_root";

/// 期数记录（与前端共享的数据结构）
#[derive(Debug, Clone, Serialize)]
pub struct StoryIssue {
    pub id: String,
    pub title: String,
    /// 所属年份目录名（根目录下直接放 PDF 时为空串）
    pub year: String,
    /// PDF 完整路径
    pub path: String,
    pub page_count: i64,
    pub size: i64,
    /// 阅读进度：当前跨页的第一页页码（0 起；封面单页为 0）
    pub reading_progress: i64,
    pub sort_order: i64,
    /// 内容指纹（文件大小 + 前 64KB 哈希）：文件改名/移动后不变，
    /// 重扫时据此归并原记录，进度等软数据不丢；仅内部使用，不下发前端
    #[serde(skip_serializing)]
    pub fingerprint: String,
    pub updated_at: String,
}

fn row_to_issue(row: &rusqlite::Row) -> rusqlite::Result<StoryIssue> {
    Ok(StoryIssue {
        id: row.get("id")?,
        title: row.get("title")?,
        year: row.get("year")?,
        path: row.get("path")?,
        page_count: row.get("page_count")?,
        size: row.get("size")?,
        reading_progress: row.get("reading_progress")?,
        sort_order: row.get("sort_order")?,
        fingerprint: row.get("fingerprint").unwrap_or_default(),
        updated_at: row.get("updated_at").unwrap_or_default(),
    })
}

// ══════════════════════════════════════════════════════════
//  根路径（单一）
// ══════════════════════════════════════════════════════════

pub fn set_root(path: &str) -> Result<(), String> {
    crate::db::set_setting(ROOT_KEY, path)
}

pub fn get_root() -> Result<Option<String>, String> {
    crate::db::get_setting(ROOT_KEY)
}

/// 清空根路径与全部期数记录（移除导入路径用）
pub fn remove_root() -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    tx.execute("DELETE FROM storyclub_issues", [])
        .map_err(|e| format!("清理期数失败: {e}"))?;
    tx.execute("DELETE FROM meta WHERE key = ?1", params![ROOT_KEY])
        .map_err(|e| format!("删除根路径失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  期数 CRUD
// ══════════════════════════════════════════════════════════

/// 整表替换期数记录（幂等）：UPSERT 保留阅读进度，删除磁盘上已不存在的记录。
/// 期数 id = md5(路径)，文件改名/移动后 id 会变，因此入库前先按「内容指纹」归并：
/// 指纹相同（内容未变，只是路径变了）→ 沿用原记录 id，进度/封面不丢；
/// 指纹为空或未命中 → 再按路径匹配保持既有 id；均未命中才视为新文件插入。
/// 故事会只维护一个根路径（换路径时先以空列表调用清空旧记录），无需按根过滤。
pub fn replace_root_issues(issues: &[StoryIssue]) -> Result<(), String> {
    let conn = lock()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    for issue in issues {
        // 1. 内容指纹匹配（文件改名/移动后仍归并原记录）
        let by_fp: Option<String> = if !issue.fingerprint.is_empty() {
            tx.query_row(
                "SELECT id FROM storyclub_issues WHERE fingerprint = ?1 LIMIT 1",
                params![issue.fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("按指纹匹配期数失败: {e}"))?
        } else {
            None
        };
        // 2. 路径匹配（保持既有 id）；3. 均未命中则用扫描生成的 id
        let id = match by_fp {
            Some(id) => id,
            None => tx
                .query_row(
                    "SELECT id FROM storyclub_issues WHERE path = ?1",
                    params![issue.path],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("按路径匹配期数失败: {e}"))?
                .unwrap_or_else(|| issue.id.clone()),
        };
        tx.execute(
            r#"INSERT INTO storyclub_issues (id, title, year, path, page_count, size, reading_progress, sort_order, updated_at, fingerprint)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, datetime('now'), ?7)
               ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 year = excluded.year,
                 path = excluded.path,
                 size = excluded.size,
                 fingerprint = CASE WHEN excluded.fingerprint <> '' THEN excluded.fingerprint ELSE storyclub_issues.fingerprint END,
                 updated_at = datetime('now')"#,
            params![
                id,
                issue.title,
                issue.year,
                issue.path,
                issue.page_count,
                issue.size,
                issue.fingerprint,
            ],
        )
        .map_err(|e| format!("写入期数失败: {e}"))?;
    }
    // 删除磁盘上已不存在的记录：路径与指纹均未命中才删（改名文件的旧指纹已在上方归并保留）
    let paths: Vec<String> = issues.iter().map(|i| i.path.clone()).collect();
    let fps: Vec<String> = issues
        .iter()
        .filter(|i| !i.fingerprint.is_empty())
        .map(|i| i.fingerprint.clone())
        .collect();
    if paths.is_empty() {
        tx.execute("DELETE FROM storyclub_issues", [])
            .map_err(|e| format!("清理旧记录失败: {e}"))?;
    } else if fps.is_empty() {
        let placeholders = paths.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM storyclub_issues WHERE path NOT IN ({placeholders})");
        tx.execute(&sql, rusqlite::params_from_iter(paths))
            .map_err(|e| format!("清理旧记录失败: {e}"))?;
    } else {
        let path_ph = paths.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let fp_ph = fps.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "DELETE FROM storyclub_issues WHERE path NOT IN ({path_ph}) AND (fingerprint = '' OR fingerprint NOT IN ({fp_ph}))"
        );
        tx.execute(&sql, rusqlite::params_from_iter(paths.iter().chain(fps.iter())))
            .map_err(|e| format!("清理旧记录失败: {e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 全部期数（年份 + 期数自然排序）
pub fn list_issues() -> Result<Vec<StoryIssue>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM storyclub_issues")
        .map_err(|e| format!("查询期数失败: {e}"))?;
    let rows = stmt
        .query_map([], row_to_issue)
        .map_err(|e| format!("查询期数失败: {e}"))?;
    let mut issues = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取期数失败: {e}"))?;
    issues.sort_by_key(|i| (natural_key(&i.year), i.sort_order, natural_key(&i.title)));
    Ok(issues)
}

/// 尚未解析页数的期数（page_count = 0；后台页数解析用，每批最多 limit 条）。
/// 解析失败标记的 -1 不会被重复拉取；前端打开阅读器成功会覆盖回写真实页数。
pub fn list_issues_missing_page_count(limit: usize) -> Result<Vec<StoryIssue>, String> {
    let conn = lock()?;
    let mut stmt = conn
        .prepare("SELECT * FROM storyclub_issues WHERE page_count = 0 LIMIT ?1")
        .map_err(|e| format!("查询期数失败: {e}"))?;
    let rows = stmt
        .query_map(params![limit as i64], row_to_issue)
        .map_err(|e| format!("查询期数失败: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取期数失败: {e}"))
}

pub fn get_issue(issue_id: &str) -> Result<Option<StoryIssue>, String> {
    let conn = lock()?;
    conn.query_row(
        "SELECT * FROM storyclub_issues WHERE id = ?1",
        params![issue_id],
        row_to_issue,
    )
    .optional()
    .map_err(|e| format!("查询期数失败: {e}"))
}

pub fn set_reading_progress(issue_id: &str, page: i64) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "UPDATE storyclub_issues SET reading_progress = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![page, issue_id],
    )
    .map_err(|e| format!("保存进度失败: {e}"))?;
    Ok(())
}

/// 回写期数页数（打开阅读器时由前端 pdf.js 解析后写入），幂等
pub fn set_page_count(issue_id: &str, count: i64) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "UPDATE storyclub_issues SET page_count = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![count, issue_id],
    )
    .map_err(|e| format!("更新页数失败: {e}"))?;
    Ok(())
}
