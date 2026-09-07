//! TXT 小说解析与按需读取（只读，绝不修改原文件）
//!
//! - 编码检测：UTF-8（含 BOM）优先，否则 GB18030（覆盖 GBK/GB2312，兼容中文 TXT 老文件）
//! - 章节识别：按行匹配常见章节标题（第X章/回/节/卷/集/部/篇/话、Chapter N、序章/楔子/番外/尾声/后记/终章等）
//! - 章节以「原始文件字节区间」存储：读取某章只需读取该区间并解码，
//!   几十 MB 的大书也能毫秒级切换章节，满足秒开要求
//! - 找不到任何章节标题时按行对齐切块兜底，避免整本变成一章导致前端渲染卡死
//!
//! 参考：fish-book 的连续阅读模型与编码检测思路；这里采用更优的章节化 + 字节区间方案。

use encoding_rs::{Encoding, GB18030, UTF_16BE, UTF_16LE, UTF_8};
use std::path::Path;

/// 单章：原始文件字节区间（offset 起，len 长；区间含章节末尾换行，读取时清理）
#[derive(Debug, Clone)]
pub struct TxtChapter {
    pub title: String,
    pub offset: u64,
    pub len: u64,
}

#[derive(Debug, Clone)]
pub struct TxtMeta {
    pub title: String,
    pub author: String,
    /// 编码标签（写入 chapters_json 的 href，读取时据此解码）
    pub encoding: &'static str,
    pub chapters: Vec<TxtChapter>,
}

/// 编码标签 → Encoding
fn encoding_of(tag: &str) -> &'static Encoding {
    match tag {
        "utf-8" => UTF_8,
        "utf-16le" => UTF_16LE,
        "utf-16be" => UTF_16BE,
        _ => GB18030,
    }
}

fn encoding_tag(enc: &'static Encoding) -> &'static str {
    if enc == UTF_8 {
        "utf-8"
    } else if enc == UTF_16LE {
        "utf-16le"
    } else if enc == UTF_16BE {
        "utf-16be"
    } else {
        "gb18030"
    }
}

/// 检测编码：BOM > 合法 UTF-8 > GB18030（GBK/GB2312 由 GB18030 覆盖）
fn detect_encoding(raw: &[u8]) -> &'static Encoding {
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }
    if raw.starts_with(&[0xFF, 0xFE]) {
        return UTF_16LE;
    }
    if raw.starts_with(&[0xFE, 0xFF]) {
        return UTF_16BE;
    }
    if std::str::from_utf8(raw).is_ok() {
        return UTF_8;
    }
    GB18030
}

/// 中文数字
const CN_NUM: &str = "零〇一二三四五六七八九十百千万亿两";
/// 章节单位
const CN_SUFFIX: &str = "章节回卷集部篇话";
/// 独立成行的章节词（允许后面跟数字/卷册词缀，如「番外篇」「后记2」）
const STANDALONE: &[&str] = &[
    "序章", "楔子", "序言", "前言", "引子", "引文",
    "上卷", "中卷", "下卷", "番外", "尾声", "后记", "终章",
    "结局", "完结", "完本", "外传", "特别篇", "最终章", "大结局",
];

/// 判断一行是否为章节标题
fn is_chapter_heading(line: &str) -> bool {
    let t = line.trim();
    // 快速预过滤：标题几乎都以「第」「Chapter」「序楔前引上中下番尾后终结完外特最」开头
    let first = t.chars().next().unwrap_or(' ');
    if first != '第'
        && first != 'C'
        && first != 'c'
        && !"序楔前引上中下番尾后终结完外特最".contains(first)
    {
        return false;
    }
    let char_count = t.chars().count();
    if t.is_empty() || char_count > 60 {
        return false;
    }
    // 第X章 / 回 / 节 / 卷 / 集 / 部 / 篇 / 话（数字可用阿拉伯/中文数字，可带空格）
    if let Some(rest) = t.strip_prefix('第') {
        let mut seen = false;
        for c in rest.chars() {
            if c.is_ascii_digit() || CN_NUM.contains(c) || c == ' ' {
                seen = true;
                continue;
            }
            return seen && CN_SUFFIX.contains(c);
        }
        return false;
    }
    // Chapter N（大小写不敏感）
    if let Some(rest) = t.to_ascii_lowercase().strip_prefix("chapter") {
        let rest = rest.trim_start();
        return rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit());
    }
    // 独立章节词：精确匹配，或词后仅跟数字 / 卷册词缀（如「番外1」「后记二」「结局篇」），
    // 避免把正文中偶发以该词开头的行误判（如「正文内容」）
    for w in STANDALONE {
        if t.starts_with(w) {
            let rest = t[w.len()..].trim();
            if rest.is_empty() {
                return true;
            }
            if rest.chars().count() <= 4
                && rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || CN_NUM.contains(c) || "篇集话部回上中下".contains(c))
            {
                return true;
            }
        }
    }
    false
}

/// 解析 TXT 小说：检测编码、识别章节、返回字节区间
pub fn parse_txt(path: &Path) -> Result<TxtMeta, String> {
    let raw = std::fs::read(path).map_err(|e| format!("读取 TXT 失败: {e}"))?;
    if raw.is_empty() {
        return Err("TXT 文件为空".to_string());
    }
    let encoding = detect_encoding(&raw);
    let enc_tag = encoding_tag(encoding);

    // UTF-16（极罕见）→ 整本作为一章，避免按 \n 字节切分 UTF-16 出错
    if encoding == UTF_16LE || encoding == UTF_16BE {
        let text = encoding.decode(&raw).0.into_owned();
        let mut title_hint = String::new();
        for l in text.lines() {
            let t = l.trim();
            if !t.is_empty() {
                if t.chars().count() <= 40 && !is_chapter_heading(t) {
                    title_hint = t.to_string();
                }
                break;
            }
        }
        let title = if title_hint.is_empty() {
            file_stem_title(path)
        } else {
            title_hint
        };
        return Ok(TxtMeta {
            title,
            author: String::new(),
            encoding: enc_tag,
            chapters: vec![TxtChapter {
                title: "全文".to_string(),
                offset: 0,
                len: raw.len() as u64,
            }],
        });
    }

    let mut chapters: Vec<TxtChapter> = Vec::new();
    let mut title_hint = String::new();
    let mut author_hint = String::new();
    let mut pending_start = 0usize;
    let mut pending_title = String::new();
    let n = raw.len();
    let mut head_lines: Vec<String> = Vec::new(); // 开头若干非空行，用于书名/作者识别

    // 按行扫描（原始字节按 \n 切分；\r\n 时 \r 归入上一行区间，读取时清理）
    let mut i = 0usize;
    let mut line_start = 0usize;
    while i <= n {
        let is_end = i == n;
        if is_end || raw[i] == b'\n' {
            let line_end = if !is_end && i > 0 && raw[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            let line = decode_line(encoding, &raw[line_start..line_end]);
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                // 书名/作者：只看开头若干非空行
                if head_lines.len() < 12 {
                    head_lines.push(trimmed.to_string());
                }
                if is_chapter_heading(trimmed) {
                    if !pending_title.is_empty() {
                        chapters.push(TxtChapter {
                            title: pending_title,
                            offset: pending_start as u64,
                            len: (line_start - pending_start) as u64,
                        });
                    }
                    pending_start = line_start;
                    pending_title = trimmed.to_string();
                }
            }
            line_start = i + 1;
        }
        i += 1;
    }
    if !pending_title.is_empty() {
        chapters.push(TxtChapter {
            title: pending_title,
            offset: pending_start as u64,
            len: (n - pending_start) as u64,
        });
    }

    // 书名/作者识别
    // 广告/样板行特征（很多 TXT 头部会贴下载广告/声明，不能当书名）
    let is_ad = |t: &str| {
        t.contains("://")
            || t.contains("www.")
            || t.contains(".com")
            || t.contains(".zip")
            || t.contains("下载")
            || t.contains("精校")
            || t.contains("更多")
            || t.contains("关注")
            || t.contains("公众号")
            || t.contains("声明")
            || t.contains("敬告")
            || t.contains("本书来自")
            || t.contains("仅供")
            || t.contains("版权归")
            || t.contains("收藏")
            || t.contains("整理")
    };
    // 优先：头部行里含《书名》
    for l in &head_lines {
        let t = l.trim();
        if let Some(open) = t.find('《') {
            if let Some(rel) = t[open..].find('》') {
                // open 指向《（3 字节）起点；跳过其完整长度再取书名
                let inner = t[open + '《'.len_utf8()..open + rel].trim();
                if !inner.is_empty() && inner.chars().count() <= 30 {
                    title_hint = inner.to_string();
                    break;
                }
            }
        }
    }
    // 其次：形如「书名：xxx」的行（直接取冒号后的内容）
    if title_hint.is_empty() {
        for l in &head_lines {
            let t = l.trim();
            if let Some(idx) = t.find('：').or_else(|| t.find(':')) {
                let head = t[..idx].trim();
                if head == "书名" {
                    let colon_len = t[idx..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    let v = t[idx + colon_len..].trim();
                    if !v.is_empty() && v.chars().count() <= 40 {
                        title_hint = v.to_string();
                        break;
                    }
                }
            }
        }
    }
    // 再次：首个正常短行（非广告/非章节标题/非书名行/非装饰线/非作者行）
    for l in &head_lines {
        let t = l.trim();
        let decorative = !t.is_empty() && t.chars().all(|c| "-=*~·—. ".contains(c));
        if title_hint.is_empty()
            && !t.is_empty()
            && t.chars().count() <= 40
            && !is_chapter_heading(t)
            && !t.contains('《')
            && !t.starts_with("作者")
            && !t.starts_with("书名")
            && !is_ad(t)
            && !decorative
        {
            title_hint = t.to_string();
            break;
        }
    }
    // 作者：形如「作者：xxx」的行
    for l in &head_lines {
        let t = l.trim();
        if author_hint.is_empty() {
            if let Some(idx) = t.find('：').or_else(|| t.find(':')) {
                let head = t[..idx].trim();
                if head == "作者" {
                    // 冒号可能是多字节（全角：3 字节）或 ASCII（1 字节），跳过其完整长度
                    let colon_len = t[idx..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    let a = t[idx + colon_len..]
                        .trim()
                        .trim_matches(|c| c == '《' || c == '》')
                        .trim();
                    if !a.is_empty() {
                        author_hint = a.to_string();
                    }
                }
            }
        }
        if !author_hint.is_empty() {
            break;
        }
    }

    // 无章节标题 → 按行对齐切块兜底
    if chapters.is_empty() {
        const CHUNK_BYTES: usize = 64 * 1024;
        let mut start = 0usize;
        let mut idx = 1usize;
        while start < n {
            let mut end = (start + CHUNK_BYTES).min(n);
            if end < n {
                while end < n && raw[end] != b'\n' {
                    end += 1;
                }
                if end < n {
                    end += 1;
                }
            }
            chapters.push(TxtChapter {
                title: format!("第 {idx} 节"),
                offset: start as u64,
                len: (end - start) as u64,
            });
            start = end;
            idx += 1;
        }
    }

    let title = if title_hint.is_empty() {
        file_stem_title(path)
    } else {
        title_hint
    };

    Ok(TxtMeta {
        title,
        author: author_hint,
        encoding: enc_tag,
        chapters,
    })
}

/// 解码一行（GB18030 等需要逐行解码；UTF-8 用 from_utf8 快速路径）
fn decode_line<'a>(encoding: &'static Encoding, raw: &'a [u8]) -> std::borrow::Cow<'a, str> {
    if encoding == UTF_8 {
        std::str::from_utf8(raw).map(std::borrow::Cow::Borrowed).unwrap_or_default()
    } else {
        encoding.decode(raw).0
    }
}

/// 读取某章正文（按字节区间读取并解码；去掉开头的章节标题行）
pub fn read_chapter(path: &Path, encoding_tag: &str, offset: u64, len: u64) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    if len == 0 || len > 512 * 1024 * 1024 {
        return Err("章节区间异常".to_string());
    }
    let mut file = std::fs::File::open(path).map_err(|e| format!("打开 TXT 失败: {e}"))?;
    let mut buf = vec![0u8; len as usize];
    file.seek(SeekFrom::Start(offset)).map_err(|e| format!("定位章节失败: {e}"))?;
    file.read_exact(&mut buf).map_err(|e| format!("读取章节失败: {e}"))?;
    let enc = encoding_of(encoding_tag);
    let text = enc.decode(&buf).0.into_owned();
    let mut body = text.as_str();
    if let Some(pos) = body.find('\n') {
        let first = body[..pos].trim();
        if is_chapter_heading(first) {
            body = &body[pos + 1..];
        }
    }
    Ok(body.trim().to_string())
}

fn file_stem_title(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // 去《》与（xx版）作者：xx 等文件名后缀
    let mut t = stem.trim();
    if t.starts_with('《') && t.contains('》') {
        if let Some(end) = t.find('》') {
            t = t[1..end].trim();
        }
    }
    if let Some(idx) = t.find('（').or_else(|| t.find('(')) {
        t = t[..idx].trim();
    }
    t.to_string()
}

// ══════════════════════════════════════════════════════════
//  测试
// ══════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_txt(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("lzy_novel_txt_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn detect_utf8_vs_gbk() {
        assert_eq!(detect_encoding("你好世界".as_bytes()), UTF_8);
        // GBK 编码的“第一章 测试”
        let gbk = [0xB5, 0xDA, 0xD2, 0xBB, 0xD5, 0xC2, 0x20, 0xB2, 0xE2, 0xCA, 0xD4];
        assert_eq!(detect_encoding(&gbk), GB18030);
    }

    #[test]
    fn parse_chapters_utf8() {
        let content = "《测试小说》\n作者：张三\n第一章 开始\n这是正文。\n第二章 继续\n更多内容。\n第三章 结束\n结尾。\n";
        let p = tmp_txt("a.txt", content);
        let meta = parse_txt(&p).unwrap();
        assert_eq!(meta.title, "测试小说"); // 《》书名号已去除
        assert_eq!(meta.author, "张三");
        assert_eq!(meta.chapters.len(), 3); // 前言（书名/作者）不作为章节，直接 3 章
        assert_eq!(meta.chapters[0].title, "第一章 开始");
        let body = read_chapter(&p, meta.encoding, meta.chapters[0].offset, meta.chapters[0].len).unwrap();
        assert!(body.contains("这是正文。"));
        assert!(!body.contains("第一章"));
    }

    #[test]
    fn parse_gbk_chapters() {
        // 直接写原始 GBK 字节（不能用 from_utf8_lossy，它会替换非法序列）
        let mut gbk = Vec::new();
        gbk.extend_from_slice(&[0xB2, 0xE2, 0xCA, 0xD4, 0xB0, 0xCD]); // 测试吧(GBK)
        gbk.extend_from_slice(&[0x0A]);
        gbk.extend_from_slice(&[0xB5, 0xDA, 0x31, 0xD5, 0xC2]); // 第1章(GBK)
        gbk.extend_from_slice(&[0x0A]);
        gbk.extend_from_slice(&[0xD5, 0xFD, 0xCE, 0xC4, 0xC4, 0xDA, 0xC8, 0xDD]); // 正文内容(GBK)
        gbk.extend_from_slice(&[0x0A]);
        let dir = std::env::temp_dir().join("lzy_novel_txt_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("gbk.txt");
        std::fs::write(&p, &gbk).unwrap();
        let meta = parse_txt(&p).unwrap();
        assert_eq!(meta.encoding, "gb18030");
        assert!(meta.chapters.iter().any(|c| c.title.starts_with("第1章")));
        // 读取该章正文应能正确解码
        let ch = meta.chapters.iter().find(|c| c.title.starts_with("第1章")).unwrap();
        eprintln!("debug: chapter={ch:?} file_len={}", std::fs::metadata(&p).unwrap().len());
        let body = read_chapter(&p, meta.encoding, ch.offset, ch.len).unwrap();
        eprintln!("debug: body={body:?}");
        assert!(body.contains("正文内容"), "GBK 解码失败: {body:?}");
    }

    #[test]
    fn fallback_chunking() {
        let content = "没有章节标题的一段很长很长的文字，只能靠切块。\n".repeat(5000);
        let p = tmp_txt("chunk.txt", &content);
        let meta = parse_txt(&p).unwrap();
        assert!(meta.chapters.len() >= 1);
        assert!(meta.chapters[0].title.starts_with("第 1 节"));
    }
}
