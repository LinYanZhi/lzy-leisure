//! 故事会期数页数后台解析：扫描后异步静默解析 page_count=0 的期数（增量），
//! 写库后 emit `storyclub-pages-done` 事件，前端刷新显示真实页数。
//!
//! 不阻塞扫描命令与书架浏览（解析在独立线程跑，写库逐条短锁）；
//! 已解析 / 解析失败（记为 -1）的期数下次不再处理，前端打开阅读器成功会覆盖回写真实页数。
use crate::db::storyclub as story_db;
use std::sync::atomic::{AtomicBool, Ordering};

/// 单批处理条数（避免一次性占满内存；写库每批释放锁）
const BATCH: usize = 20;
/// 单次任务最多处理期数：分散到多次书架访问逐步补齐，
/// 避免一次性长时间全量解析（lopdf 需读入整个 PDF）占满磁盘 IO 拖卡页面
const BUDGET: usize = 50;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// 启动后台页数解析（已在跑则忽略；扫描完成后调用）
pub fn start() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let result = run();
        RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(count) if count > 0 => {
                crate::events::emit(
                    "storyclub-pages-done",
                    serde_json::json!({ "parsed": count }),
                );
            }
            Ok(_) => {}
            Err(e) => log::warn!("故事会页数后台解析失败: {e}"),
        }
    });
}

fn run() -> Result<usize, String> {
    let mut total = 0;
    loop {
        let pending = story_db::list_issues_missing_page_count(BATCH)?;
        if pending.is_empty() || total >= BUDGET {
            break;
        }
        for issue in pending {
            if total >= BUDGET {
                break;
            }
            match pdf_page_count(&issue.path) {
                Some(p) if p > 0 => {
                    let _ = story_db::set_page_count(&issue.id, p);
                    total += 1;
                }
                // 损坏/不可解析的 PDF 记为 -1，后续不再反复处理
                _ => {
                    let _ = story_db::set_page_count(&issue.id, -1);
                }
            }
        }
        // 批间短暂喘息，避免持续高 IO 影响书架浏览
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(total)
}

/// 解析 PDF 页数（seek 式只读页面树，不整读 PDF；失败回退 lopdf 整读兜底）
fn pdf_page_count(path: &str) -> Option<i64> {
    if let Ok(n) = crate::story_first_page::page_count(std::path::Path::new(path)) {
        return Some(n);
    }
    let doc = lopdf::Document::load(path).ok()?;
    let n = doc.get_pages().len();
    if n == 0 {
        None
    } else {
        Some(n as i64)
    }
}
