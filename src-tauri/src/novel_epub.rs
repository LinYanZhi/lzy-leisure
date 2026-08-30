//! EPUB 解析（只读，绝不修改原文件）
//!
//! 从 EPUB（zip 容器）中读取元数据、章节列表、章节文本与内嵌封面：
//!   - META-INF/container.xml → OPF 包文件
//!   - OPF → metadata（dc:title / dc:creator）、manifest（id→href/media-type）、spine（章节顺序）
//!   - 章节标题：优先取 spine 文档内的 <title>，兜底用 href 文件名
//!   - 封面：manifest 中 role=cover 或 id/href 含 cover 的图片项
//!   - 章节正文：解压 xhtml → 轻量 HTML→纯文本转换（去标签/解码实体/段落换行）
//!
//! 不引入 XML/HTML 解析依赖：EPUB 内部结构相对固定，用轻量扫描即可可靠覆盖；
//! 解析失败一律返回 Err，由调用方降级（不阻塞主流程）。
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// EPUB 解析结果（元数据 + 章节列表 + 可选封面）
#[derive(Debug, Clone, Serialize)]
pub struct EpubMeta {
    pub title: String,
    pub author: String,
    /// 按 spine 顺序的章节（href 为包内相对路径）
    pub chapters: Vec<EpubChapter>,
    /// 内嵌封面（若有）
    pub cover: Option<EpubCover>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpubChapter {
    pub href: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpubCover {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// 解析 EPUB 文件（path 为磁盘上的 .epub 文件）
pub fn parse_epub(path: &Path) -> Result<EpubMeta, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打开 EPUB 失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("解压 EPUB 失败: {e}"))?;

    // 1. container.xml → OPF 路径
    let container = read_zip(&mut zip, "META-INF/container.xml")
        .map_err(|e| format!("读取 container.xml 失败: {e}"))?;
    let container = String::from_utf8_lossy(&container).into_owned();
    let opf_path = container_opf_path(&container).ok_or_else(|| "EPUB 缺少 OPF 定义".to_string())?;
    // OPF 路径可能含 URL 编码（如 %20），zip 内存储名为原始字节，先尝试原样读取
    let opf = read_zip(&mut zip, &opf_path)
        .or_else(|_| read_zip(&mut zip, &percent_decode(&opf_path)))
        .map_err(|e| format!("读取 OPF 失败: {e}"))?;
    let opf = String::from_utf8_lossy(&opf).into_owned();

    // 2. OPF → 元数据 / manifest / spine
    let base_dir = opf_base_dir(&opf_path);
    let (title, author) = opf_metadata(&opf);
    let items = opf_items(&opf);
    let spine = opf_spine(&opf);

    // 3. 章节列表：spine 顺序 → manifest href
    let mut chapters = Vec::new();
    for idref in spine {
        if let Some(item) = items.get(&idref) {
            // 只收 xhtml/html 内容
            if item.media_type.contains("html")
                || item.href.ends_with(".xhtml")
                || item.href.ends_with(".html")
            {
                chapters.push(EpubChapter {
                    href: join_rel(&base_dir, &item.href),
                    title: String::new(),
                });
            }
        }
    }
    if chapters.is_empty() {
        return Err("EPUB 中没有可读章节".to_string());
    }

    // 4. 章节标题：读每个 spine 文档的 <title>（失败兜底用文件名）
    for (idx, ch) in chapters.iter_mut().enumerate() {
        let raw = read_zip(&mut zip, &ch.href)
            .or_else(|_| read_zip(&mut zip, &percent_decode(&ch.href)))
            .unwrap_or_default();
        let html = String::from_utf8_lossy(&raw).into_owned();
        let t = extract_title(&html).unwrap_or_default();
        ch.title = if t.trim().is_empty() {
            ch.href
                .rsplit('/')
                .next()
                .map(|s| s.trim_end_matches(".xhtml").trim_end_matches(".html").to_string())
                .unwrap_or_else(|| format!("第{}节", idx + 1))
        } else {
            t.trim().to_string()
        };
    }

    // 5. 封面：metadata cover id → manifest；兜底 id/href 含 cover 的图片项
    let cover = opf_cover(&opf)
        .and_then(|cid| items.get(&cid).cloned())
        .or_else(|| {
            items
                .iter()
                .find(|(id, it)| it.media_type.starts_with("image/") && id.contains("cover"))
                .map(|(_, it)| it.clone())
                .or_else(|| {
                    items
                        .values()
                        .find(|it| {
                            it.media_type.starts_with("image/")
                                && it.href.to_ascii_lowercase().contains("cover")
                        })
                        .cloned()
                })
        });
    let cover = cover.and_then(|it| {
        let p = join_rel(&base_dir, &it.href);
        let bytes = read_zip(&mut zip, &p)
            .or_else(|_| read_zip(&mut zip, &percent_decode(&p)))
            .ok()?;
        Some(EpubCover {
            mime: it.media_type.clone(),
            bytes,
        })
    });

    Ok(EpubMeta {
        title: title.trim().to_string(),
        author: author.trim().to_string(),
        chapters,
        cover,
    })
}

/// 读取某章节的纯文本（href 为 parse_epub 返回的包内路径）
pub fn read_chapter_text(path: &Path, href: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打开 EPUB 失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("解压 EPUB 失败: {e}"))?;
    let raw = read_zip(&mut zip, href)
        .or_else(|_| read_zip(&mut zip, &percent_decode(href)))
        .map_err(|e| format!("读取章节失败: {e}"))?;
    let html = String::from_utf8_lossy(&raw).into_owned();
    Ok(html_to_text(&html))
}

// ══════════════════════════════════════════════════════════
//  zip 辅助
// ══════════════════════════════════════════════════════════

fn read_zip(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Vec<u8>, String> {
    let mut entry = zip
        .by_name(name)
        .map_err(|_| format!("包内文件不存在: {name}"))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).map_err(|e| format!("读取 {name} 失败: {e}"))?;
    Ok(buf)
}

/// 简易 percent 解码（EPUB 内 href 常用 %20 编码空格等）
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ══════════════════════════════════════════════════════════
//  XML 轻量解析（针对 EPUB 固定结构）
// ══════════════════════════════════════════════════════════

/// 单个开标签：名 + 属性
struct XmlTag<'a> {
    name: &'a str,
    attrs: HashMap<String, String>,
}

/// 扫描全部开标签（跳过注释 / 闭合标签 / 自闭合），返回 (名, 属性表)
fn parse_tags(xml: &str) -> Vec<XmlTag<'_>> {
    let mut tags = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // 注释跳过
        if xml[i..].starts_with("<!--") {
            if let Some(end) = xml[i + 4..].find("-->") {
                i += 4 + end + 3;
            } else {
                break;
            }
            continue;
        }
        if let Some(end) = xml[i + 1..].find('>') {
            let raw = &xml[i + 1..i + 1 + end];
            let trimmed = raw.trim();
            // 跳过闭合标签、声明、处理指令、CDATA
            if !trimmed.is_empty()
                && !trimmed.starts_with('/')
                && !trimmed.starts_with('?')
                && !trimmed.starts_with('!')
            {
                // 去掉自闭合尾部斜杠
                let body = trimmed.trim_end_matches('/').trim_end();
                let mut parts = body.splitn(2, char::is_whitespace);
                let name = parts.next().unwrap_or("").trim();
                let mut attrs = HashMap::new();
                if let Some(rest) = parts.next() {
                    parse_attrs(rest, &mut attrs);
                }
                tags.push(XmlTag { name, attrs });
            }
            i += 1 + end + 1;
        } else {
            break;
        }
    }
    tags
}

fn parse_attrs(rest: &str, attrs: &mut HashMap<String, String>) {
    let bytes = rest.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        // 跳过空白
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= n {
            break;
        }
        // 属性名：到 = 或空白
        let name_start = i;
        while i < n && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = &rest[name_start..i];
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n || bytes[i] != b'=' {
            continue;
        }
        i += 1; // '='
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            i += 1;
            let val_start = i;
            while i < n && bytes[i] != quote {
                i += 1;
            }
            let val = &rest[val_start..i];
            if i < n {
                i += 1;
            }
            attrs.insert(name.to_ascii_lowercase(), percent_decode(val));
        } else {
            let val_start = i;
            while i < n && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            attrs.insert(name.to_ascii_lowercase(), percent_decode(&rest[val_start..i]));
        }
    }
}

/// 取标签文本内容（如 <dc:title>xxx</dc:title>），大小写不敏感
fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let open = format!("<{tag}");
    let idx = lower.find(&open)?;
    let after = &xml[idx..];
    let close = format!("</{tag}>");
    let close_lower = lower[idx..].find(&close)?;
    Some(after[..close_lower].to_string())
}

fn container_opf_path(container: &str) -> Option<String> {
    parse_tags(container)
        .into_iter()
        .find(|t| t.name.to_ascii_lowercase() == "rootfile")
        .and_then(|t| t.attrs.get("full-path").cloned())
}

fn opf_metadata(opf: &str) -> (String, String) {
    let title = tag_text(opf, "dc:title")
        .as_deref()
        .map(html_to_text)
        .unwrap_or_default();
    let author = tag_text(opf, "dc:creator")
        .as_deref()
        .map(html_to_text)
        .unwrap_or_default();
    (title, author)
}

#[derive(Clone)]
struct OpfItem {
    href: String,
    media_type: String,
}

fn opf_items(opf: &str) -> HashMap<String, OpfItem> {
    let mut map = HashMap::new();
    for t in parse_tags(opf) {
        if t.name.to_ascii_lowercase() != "item" {
            continue;
        }
        if let (Some(id), Some(href)) = (t.attrs.get("id"), t.attrs.get("href")) {
            map.insert(
                id.clone(),
                OpfItem {
                    href: href.clone(),
                    media_type: t
                        .attrs
                        .get("media-type")
                        .cloned()
                        .unwrap_or_default(),
                },
            );
        }
    }
    map
}

fn opf_spine(opf: &str) -> Vec<String> {
    let mut order = Vec::new();
    for t in parse_tags(opf) {
        if t.name.to_ascii_lowercase() == "itemref" {
            if let Some(idref) = t.attrs.get("idref") {
                order.push(idref.clone());
            }
        }
    }
    order
}

/// metadata 里 <meta name="cover" content="id"/> 指定的封面项 id
fn opf_cover(opf: &str) -> Option<String> {
    for t in parse_tags(opf) {
        if t.name.to_ascii_lowercase() != "meta" {
            continue;
        }
        let name = t.attrs.get("name").map(|s| s.to_ascii_lowercase());
        if name.as_deref() == Some("cover") {
            return t.attrs.get("content").cloned();
        }
    }
    None
}

fn opf_base_dir(opf_path: &str) -> String {
    match opf_path.rfind('/') {
        Some(i) => opf_path[..i].to_string(),
        None => String::new(),
    }
}

/// 相对路径拼接：href 相对 OPF 所在目录
fn join_rel(base_dir: &str, href: &str) -> String {
    if base_dir.is_empty() {
        href.to_string()
    } else {
        format!("{base_dir}/{href}")
    }
}

// ══════════════════════════════════════════════════════════
//  HTML → 纯文本
// ══════════════════════════════════════════════════════════

/// 提取文档 <title>（章节标题用），并转为纯文本
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let after_open = lower[open..].find('>')?;
    let start = open + after_open + 1;
    let close = lower[start..].find("</title")?;
    Some(html_to_text(&html[start..start + close]))
}

/// 轻量 HTML → 纯文本：去标签、解码实体、块级元素换行
fn html_to_text(html: &str) -> String {
    // 先按字符级扫描，块级标签前后补换行
    let mut out = String::new();
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut skip = 0u32; // script/style 深度
    let block = [
        "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr",
        "blockquote", "section", "article", "pre", "table",
    ];
    while i < n {
        if bytes[i] == b'<' {
            if html[i..].starts_with("<!--") {
                if let Some(end) = html[i + 4..].find("-->") {
                    i += 4 + end + 3;
                    continue;
                }
            }
            if let Some(end) = html[i + 1..].find('>') {
                let raw = &html[i + 1..i + 1 + end];
                let trimmed = raw.trim();
                let is_close = trimmed.starts_with('/');
                let name = if is_close {
                    trimmed[1..].trim().split_whitespace().next().unwrap_or("")
                } else {
                    trimmed.split_whitespace().next().unwrap_or("")
                };
                let name = name.trim_end_matches('/').to_ascii_lowercase();
                if !is_close && (name == "script" || name == "style") {
                    skip += 1;
                    i += 1 + end + 1;
                    continue;
                }
                if skip > 0 {
                    if is_close && (name == "script" || name == "style") {
                        skip -= 1;
                    }
                } else if block.contains(&name.as_str()) {
                    push_line(&mut out);
                }
                i += 1 + end + 1;
                continue;
            }
        }
        if skip == 0 && bytes[i] == b'&' {
            if let Some(semi) = html[i + 1..].find(';') {
                let ent = &html[i + 1..i + 1 + semi];
                if let Some(c) = decode_entity(ent) {
                    out.push(c);
                    i += 1 + semi + 1;
                    continue;
                }
            }
        }
        if skip == 0 {
            let c = bytes[i];
            // 保留常见空白；合并换行由 push_line 控制
            if c == b'\n' || c == b'\r' || c == b'\t' {
                // 空白折叠：不直接写入，交给边界处理
                i += 1;
                continue;
            }
            // 用 char 解码（多字节 UTF-8）
            let ch = html[i..].chars().next().unwrap_or_default();
            if !ch.is_control() {
                out.push(ch);
            }
            i += ch.len_utf8();
            continue;
        }
        i += 1;
    }
    // 规范化：压缩连续空行
    let mut result = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines() {
        let l = line.trim();
        if l.is_empty() {
            blank += 1;
            if blank <= 1 {
                result.push_str("\n\n");
            }
        } else {
            blank = 0;
            result.push_str(l);
            result.push('\n');
        }
    }
    result.trim().to_string()
}

#[inline]
fn push_line(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// 解码 HTML 实体（常见命名 + 数字）
fn decode_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00a0}'),
        "ndash" => Some('–'),
        "mdash" => Some('—'),
        "hellip" => Some('…'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "ldquo" => Some('\u{201c}'),
        "rdquo" => Some('\u{201d}'),
        "middot" => Some('·'),
        "times" => Some('×'),
        "copy" => Some('©'),
        "reg" => Some('®'),
        _ => {
            if let Some(num) = ent.strip_prefix('#') {
                if let Some(n) = num.strip_prefix('x').and_then(|h| u32::from_str_radix(h, 16).ok()) {
                    char::from_u32(n)
                } else {
                    num.parse::<u32>().ok().and_then(char::from_u32)
                }
            } else {
                None
            }
        }
    }
}
