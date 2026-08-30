//! 视频目录扫描
use std::path::Path;

pub const VIDEO_EXTENSIONS: [&str; 9] = [
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "ts",
];

/// 视频种类标识（videos.kinds 数组元素，前端展示中文名）
pub const KIND_MOVIE: &str = "movie";
pub const KIND_SHORT: &str = "short";
pub const KIND_ANIME: &str = "anime";
pub const KIND_VERTICAL: &str = "vertical";

/// 由 duration 字符串（HH:MM:SS 或 MM:SS）解析秒数
fn duration_to_secs(duration: &str) -> f64 {
    let parts: Vec<&str> = duration.split(':').collect();
    let mut secs = 0.0f64;
    for (i, p) in parts.iter().enumerate() {
        let v: f64 = p.parse().unwrap_or(0.0);
        secs += v * 60f64.powi((parts.len() - 1 - i) as i32);
    }
    secs
}

/// 扫描时自动推断视频种类（可多选）：
/// - 竖屏（高 > 宽）→ vertical
/// - 时长 < 15 分钟 → short
/// - 时长 ≥ 60 分钟 → movie
/// - 路径/标题含动漫关键词 → anime
/// 推断仅供参考，可在播放器编辑面板手动增删。
pub fn infer_kinds(
    path: &str,
    title: &str,
    duration: &str,
    width: Option<i64>,
    height: Option<i64>,
) -> Vec<String> {
    let mut kinds: Vec<String> = Vec::new();
    if let (Some(w), Some(h)) = (width, height) {
        if h > w {
            kinds.push(KIND_VERTICAL.to_string());
        }
    }
    let secs = duration_to_secs(duration);
    if secs > 0.0 && secs < 15.0 * 60.0 {
        kinds.push(KIND_SHORT.to_string());
    } else if secs >= 60.0 * 60.0 {
        kinds.push(KIND_MOVIE.to_string());
    }
    const ANIME_KEYWORDS: [&str; 5] = ["动漫", "番剧", "anime", "cartoon", "動漫"];
    let hay = format!("{} {title}", path.to_lowercase());
    if ANIME_KEYWORDS.iter().any(|k| hay.contains(k)) {
        kinds.push(KIND_ANIME.to_string());
    }
    kinds
}

pub fn is_video_file(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    VIDEO_EXTENSIONS.contains(&ext.as_str())
}

/// 字幕文件扩展名（大小写不敏感）
pub const SUBTITLE_EXTENSIONS: [&str; 4] = ["ass", "srt", "ssa", "vtt"];

/// 检测视频同目录下的同名字幕文件（仅完全同名：去扩展名后与视频完全相等）。
/// 返回字幕文件完整路径；没有则 None。ass 之外多字幕时按 ass > srt > ssa > vtt 优先。
pub fn find_subtitle(video_path: &Path) -> Option<String> {
    let dir = video_path.parent()?;
    let stem = video_path.file_stem()?.to_string_lossy().to_lowercase();
    let mut best: Option<(usize, String)> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = p.file_name()?.to_string_lossy().to_string();
        let lower = name.to_lowercase();
        let Some(ext) = Path::new(&lower).extension().map(|e| e.to_string_lossy().to_string()) else {
            continue;
        };
        if !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        if Path::new(&lower).file_stem()?.to_string_lossy() != stem {
            continue;
        }
        let priority = SUBTITLE_EXTENSIONS
            .iter()
            .position(|e| *e == ext)
            .unwrap_or(usize::MAX);
        if best.as_ref().map(|(p, _)| priority < *p).unwrap_or(true) {
            best = Some((priority, p.to_string_lossy().to_string()));
        }
    }
    best.map(|(_, path)| path)
}

/// 将字幕文件字节转为 UTF-8 字符串（ass/srt 常见 UTF-8 或 GBK）
pub fn decode_subtitle(bytes: &[u8]) -> String {
    let (cow, _, _) = encoding_rs::UTF_8.decode(bytes);
    if !cow.to_string().contains('\u{FFFD}') {
        return cow.into_owned();
    }
    let (cow, _, _) = encoding_rs::GBK.decode(bytes);
    if !cow.to_string().contains('\u{FFFD}') {
        return cow.into_owned();
    }
    // 兜底：UTF-8 lossy
    String::from_utf8_lossy(bytes).into_owned()
}

/// 扫描目录下所有视频文件，返回 (绝对路径, 标题)
pub fn scan_videos(dir: &Path) -> Result<Vec<(String, String)>, String> {
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    let mut out: Vec<(String, String)> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];

    while let Some(cur) = stack.pop() {
        // 根目录读取失败视为扫描失败；子目录失败跳过（权限等不影响整棵扫描）
        let entries = match std::fs::read_dir(&cur) {
            Ok(e) => e,
            Err(e) if cur == dir => {
                return Err(format!("读取目录失败 {}: {e}", cur.display()));
            }
            Err(e) => {
                log::warn!("跳过无法读取的目录 {}: {e}", cur.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                // 跳过隐藏目录
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                stack.push(p);
            } else if p.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_video_file(&name) {
                    let title = p
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    out.push((p.to_string_lossy().to_string(), title));
                }
            }
        }
    }

    out.sort();
    Ok(out)
}

// ══════════════════════════════════════════════════════════
//  文件名解析（标题 / 年份 / 集数）
// ══════════════════════════════════════════════════════════

/// 文件名解析结果
pub struct ParsedVideoName {
    pub title: String,
    pub year: String,
    pub episode: String,
}

/// 分隔符字符（ASCII 与全角）
fn is_sep_char(c: char) -> bool {
    matches!(
        c,
        ' ' | '.'
            | '-'
            | '_'
            | '+'
            | ','
            | '，'
            | '。'
            | '('
            | ')'
            | '['
            | ']'
            | '（'
            | '）'
            | '【'
            | '】'
    )
}

/// s[..idx] 的末字符是否为分隔符（idx 须为字符边界）
fn prev_is_sep(s: &str, idx: usize) -> bool {
    s[..idx].chars().last().map(is_sep_char).unwrap_or(true)
}

/// s[idx..] 的首字符是否为分隔符（idx 须为字符边界）
fn next_is_sep(s: &str, idx: usize) -> bool {
    s[idx..].chars().next().map(is_sep_char).unwrap_or(true)
}

/// 从 s 中移除 [start, end) 区间，并修剪区间前后残留的分隔符
fn strip_span(s: &str, start: usize, end: usize) -> String {
    let mut out = String::new();
    out.push_str(s[..start].trim_end_matches(is_sep_char));
    out.push_str(s[end..].trim_start_matches(is_sep_char));
    out
}

/// 从视频文件名（不含扩展名）解析标题 / 年份 / 集数。
/// 识别规则：
///   - 集数：S01E05 / 第12话 / 第12集 / EP12 / 独立 E12（原文保留，排序时提取数值）
///   - 年份：(2020) [2020] .2020. 等 4 位年份
///   - 杂质：分辨率（1080p/4K）、编码（x264/x265/hevc/av1）、来源（webrip/bluray/dvdrip/bd）、
///     音轨（10bit）等被剔除
pub fn parse_video_filename(stem: &str) -> ParsedVideoName {
    let mut s = stem.trim().to_string();
    let mut year = String::new();
    let mut episode = String::new();

    // ── 1. 集数标识 ──
    let mut ep_span: Option<(usize, usize)> = None;
    {
        let bytes = s.as_bytes();
        let len = bytes.len();
        // 1a. S01E05 / s01e05
        let mut i = 0;
        while i + 1 < len {
            if bytes[i].eq_ignore_ascii_case(&b's') && bytes[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                while j < len && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j + 1 < len
                    && bytes[j].eq_ignore_ascii_case(&b'e')
                    && bytes[j + 1].is_ascii_digit()
                {
                    let mut k = j + 1;
                    while k < len && bytes[k].is_ascii_digit() {
                        k += 1;
                    }
                    ep_span = Some((i, k));
                }
                break;
            }
            i += 1;
        }
        // 1b. 第12话 / 第12集
        if ep_span.is_none() {
            if let Some(pos) = s.find('第') {
                let start = pos + '第'.len_utf8();
                let mut j = start;
                while j < len && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > start {
                    if let Some(c) = s[j..].chars().next() {
                        if c == '话' || c == '集' {
                            ep_span = Some((pos, j + c.len_utf8()));
                        }
                    }
                }
            }
        }
        // 1c. EP12
        if ep_span.is_none() {
            let lower = s.to_lowercase();
            if let Some(pos) = lower.find("ep") {
                let digits: usize = lower[pos + 2..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .count();
                if digits > 0 {
                    ep_span = Some((pos, pos + 2 + digits));
                }
            }
        }
        // 1d. 独立 E12（前是分隔符）
        if ep_span.is_none() {
            let mut i = 1;
            while i + 1 < len {
                if bytes[i].eq_ignore_ascii_case(&b'e')
                    && bytes[i + 1].is_ascii_digit()
                    && prev_is_sep(&s, i)
                {
                    let mut j = i + 1;
                    while j < len && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    ep_span = Some((i, j));
                    break;
                }
                i += 1;
            }
        }
    }
    if let Some((start, end)) = ep_span {
        episode = s[start..end].to_string();
        s = strip_span(&s, start, end);
    }

    // ── 2. 年份 ──
    let mut year_span: Option<(usize, usize)> = None;
    // 2a. 半角括号：(2020) / [2020]
    {
        let bytes = s.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i + 4 < len {
            let close = if bytes[i] == b'(' {
                b')'
            } else if bytes[i] == b'[' {
                b']'
            } else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < len && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - (i + 1) == 4 && j < len && bytes[j] == close {
                let y = &s[i + 1..j];
                if y.starts_with("19") || y.starts_with("20") {
                    year = y.to_string();
                    year_span = Some((i, j + 1));
                }
                break;
            }
            i += 1;
        }
    }
    // 2b. 全角括号：（2020）【2020】
    if year_span.is_none() {
        for open_c in ['（', '【'] {
            let close_c = if open_c == '（' { '）' } else { '】' };
            if let Some(pos) = s.find(open_c) {
                let start = pos + open_c.len_utf8();
                let digits: String = s[start..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if digits.len() == 4 && (digits.starts_with("19") || digits.starts_with("20")) {
                    let end = start + digits.len();
                    if s[end..].chars().next() == Some(close_c) {
                        year = digits;
                        year_span = Some((pos, end + close_c.len_utf8()));
                    }
                }
            }
        }
    }
    // 2c. 分隔符包围的年份：.2020. /  2020
    if year_span.is_none() {
        let bytes = s.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i + 4 <= len {
            let seg = &bytes[i..i + 4];
            // 仅当 4 字节全为 ASCII 数字时 i 才是安全字符边界（避免落在中文字节上切片 panic）
            if seg.iter().all(|b| b.is_ascii_digit())
                && (seg[0] == b'1' || seg[0] == b'2')
                && (i == 0 || prev_is_sep(&s, i))
                && (i + 4 == len || next_is_sep(&s, i + 4))
            {
                year = std::str::from_utf8(seg).unwrap_or("").to_string();
                year_span = Some((i, i + 4));
                break;
            }
            i += 1;
        }
    }
    if let Some((start, end)) = year_span {
        s = strip_span(&s, start, end);
    }

    // ── 3. 杂质清理（分辨率/编码/来源/音轨标记） ──
    const JUNK: [&str; 15] = [
        "1080p", "720p", "2160p", "4k", "x264", "x265", "h264", "h265", "hevc", "av1", "webrip",
        "bluray", "dvdrip", "10bit", "bd",
    ];
    loop {
        let lower = s.to_lowercase();
        let mut hit: Option<(usize, usize)> = None;
        for j in JUNK {
            if let Some(pos) = lower.find(j) {
                let end = pos + j.len();
                if (pos == 0 || prev_is_sep(&s, pos)) && (end == s.len() || next_is_sep(&s, end)) {
                    hit = Some((pos, end));
                    break;
                }
            }
        }
        match hit {
            Some((start, end)) => s = strip_span(&s, start, end),
            None => break,
        }
    }

    // ── 4. 整理：合并连续分隔符为单个空格，去除首尾 ──
    let mut title = String::new();
    let mut prev_sep = false;
    for c in s.trim_matches(is_sep_char).chars() {
        if is_sep_char(c) {
            if !prev_sep {
                title.push(' ');
                prev_sep = true;
            }
        } else {
            title.push(c);
            prev_sep = false;
        }
    }

    ParsedVideoName {
        title: title.trim().to_string(),
        year,
        episode,
    }
}
