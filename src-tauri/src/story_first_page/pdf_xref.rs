//! 故事会 PDF 最小解析器——xref 层：startxref 定位 / 经典表 / xref 流 / /Prev 链组装。
//! 原 story_first_page.rs 拆分（对象层见 pdf_objects，应用见 story_first_page）。
use super::pdf_objects::{
    decode_parms, inflate, parse_dict, png_undo_predictor, read_chunk, Obj, Parser, MAX_BLOB,
};
use std::collections::HashMap;
use std::fs::File;

/// xref 条目：对象在文件中的直接偏移，或位于对象流中
#[derive(Clone, Copy)]
pub(crate) enum Entry {
    Offset(u64),
    InObjStm(u32, u32), // 对象流对象号, 流内索引
}

/// xref 解析结果：对象映射 + 文档根引用
pub(crate) struct Xref {
    pub(crate) entries: HashMap<u32, Entry>,
    pub(crate) root: Option<(u32, u16)>,
}

/// 文件尾部找 startxref 后的绝对偏移
pub(crate) fn find_startxref(tail: &[u8]) -> Result<i64, String> {
    let kw = b"startxref";
    let mut end = tail.len();
    while end >= kw.len() {
        let mut found = None;
        for i in (0..=end - kw.len()).rev() {
            if &tail[i..i + kw.len()] == kw {
                found = Some(i);
                break;
            }
        }
        match found {
            Some(i) => {
                let mut p = Parser { buf: tail, pos: i + kw.len() };
                if let Some(v) = p.parse_int() {
                    return Ok(v);
                }
                end = i;
            }
            None => return Err("找不到 startxref".into()),
        }
    }
    Err("找不到 startxref".into())
}

/// 经典 xref 表：返回 (条目, Root, Prev)
fn parse_classic_xref(
    f: &mut File,
    pos: u64,
    file_len: u64,
) -> Result<(HashMap<u32, Entry>, Option<(u32, u16)>, Option<i64>), String> {
    let buf = read_chunk(f, pos, (4 << 20).min(file_len as usize + 1))?;
    let mut p = Parser { buf: &buf, pos: 0 };
    p.skip_ws();
    if !p.starts_with(b"xref") {
        return Err("非经典 xref 表".into());
    }
    p.pos += 4;
    let mut entries = HashMap::new();
    loop {
        p.skip_ws();
        let Some(start) = p.parse_int().filter(|v| *v >= 0) else { break };
        let Some(count) = p.parse_int().filter(|v| *v >= 0) else { break };
        for k in 0..count as usize {
            p.skip_ws();
            if p.buf.len() - p.pos < 20 {
                return Err("xref 行不完整".into());
            }
            let line = &p.buf[p.pos..p.pos + 20];
            p.pos += 20;
            let objnum = start as u32 + k as u32;
            let field1 = parse_fixed(&line[0..10]);
            if line[17] == b'n' {
                entries.insert(objnum, Entry::Offset(field1 as u64));
            }
        }
    }
    let mut root = None;
    let mut prev = None;
    p.skip_ws();
    if p.starts_with(b"trailer") {
        p.pos += 7;
        p.skip_ws();
        if p.starts_with(b"<<") {
            p.pos += 2;
            if let Ok(dict) = parse_dict(&mut p) {
                for (k, v) in dict {
                    match k.as_slice() {
                        b"Root" => root = v.as_ref(),
                        b"Prev" => prev = v.as_int(),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok((entries, root, prev))
}

/// xref 流：`N G obj <<dict>> stream ... endstream`，返回 (条目, Root, Prev)
fn parse_xref_stream(
    f: &mut File,
    pos: u64,
    file_len: u64,
) -> Result<(HashMap<u32, Entry>, Option<(u32, u16)>, Option<i64>), String> {
    let buf = read_chunk(f, pos, (4 << 20).min(file_len as usize + 1))?;
    let mut p = Parser { buf: &buf, pos: 0 };
    p.skip_ws();
    p.parse_int().ok_or("xref 流对象号缺失")?;
    p.skip_ws();
    p.parse_int().ok_or("xref 流代号缺失")?;
    p.skip_ws();
    if !p.starts_with(b"obj") {
        return Err("非 xref 流对象".into());
    }
    p.pos += 3;
    p.skip_ws();
    if !p.starts_with(b"<<") {
        return Err("xref 流缺字典".into());
    }
    p.pos += 2;
    let dict = parse_dict(&mut p)?;
    let w = dict
        .iter()
        .find(|(k, _)| k == b"W")
        .and_then(|(_, v)| match v {
            Obj::Array(a) => Some(
                a.iter()
                    .map(|o| o.as_int().unwrap_or(0) as usize)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .ok_or("xref 流缺 W")?;
    let index: Vec<u32> = match dict.iter().find(|(k, _)| k == b"Index") {
        Some((_, Obj::Array(a))) => a
            .iter()
            .filter_map(|o| o.as_int())
            .map(|i| i as u32)
            .collect(),
        _ => {
            let size = dict
                .iter()
                .find(|(k, _)| k == b"Size")
                .and_then(|(_, v)| v.as_int())
                .unwrap_or(0) as u32;
            vec![0, size]
        }
    };
    let root = dict.iter().find(|(k, _)| k == b"Root").and_then(|(_, v)| v.as_ref());
    let prev = dict.iter().find(|(k, _)| k == b"Prev").and_then(|(_, v)| v.as_int());
    let length = dict
        .iter()
        .find(|(k, _)| k == b"Length")
        .and_then(|(_, v)| v.as_int())
        .ok_or("xref 流缺 Length")?;
    let flate = dict
        .iter()
        .find(|(k, _)| k == b"Filter")
        .map(|(_, v)| v.as_name() == Some(b"FlateDecode"))
        .unwrap_or(false);
    // DecodeParms：xref 流常见 PNG 预测（Predictor 12 = PNG Up），解压后需逆预测还原
    let (predictor, columns) = decode_parms(&dict);
    p.skip_ws();
    if !p.starts_with(b"stream") {
        return Err("xref 流缺 stream".into());
    }
    p.pos += 6;
    if p.buf.get(p.pos) == Some(&b'\r') {
        p.pos += 1;
    }
    if p.buf.get(p.pos) == Some(&b'\n') {
        p.pos += 1;
    }
    let mut raw = p.buf[p.pos..].to_vec();
    if raw.len() > length as usize {
        raw.truncate(length as usize);
    }
    let mut data = if flate { inflate(&raw)? } else { raw };
    if predictor > 1 {
        data = png_undo_predictor(&data, columns)?;
    }
    if data.len() > MAX_BLOB {
        return Err("xref 流过大".into());
    }
    let row = w[0] + w[1] + w[2];
    if row == 0 {
        return Err("W 全零".into());
    }
    let mut entries = HashMap::new();
    let mut data_off = 0usize;
    for pair in index.chunks_exact(2) {
        let (s0, n) = (pair[0], pair[1]);
        for k in 0..n {
            let objnum = s0 + k;
            let off = data_off + k as usize * row;
            if off + row > data.len() {
                break;
            }
            let t = read_be(&data, off, w[0]);
            let f2 = read_be(&data, off + w[0], w[1]);
            let f3 = read_be(&data, off + w[0] + w[1], w[2]);
            match t {
                1 => entries.insert(objnum, Entry::Offset(f2 as u64)),
                2 => entries.insert(objnum, Entry::InObjStm(f2 as u32, f3 as u32)),
                _ => None,
            };
        }
        data_off += n as usize * row;
    }
    Ok((entries, root, prev))
}

fn parse_fixed(s: &[u8]) -> i64 {
    let mut v: i64 = 0;
    for &b in s {
        if b.is_ascii_digit() {
            v = v * 10 + (b - b'0') as i64;
        }
    }
    v
}

fn read_be(data: &[u8], off: usize, width: usize) -> i64 {
    if width == 0 || off + width > data.len() {
        return 0;
    }
    let mut v: i64 = 0;
    for &b in &data[off..off + width] {
        v = (v << 8) | b as i64;
    }
    v
}

/// 组装 xref：沿 /Prev 链收集各段，新段（先解析的）覆盖旧段
pub(crate) fn build_xref(f: &mut File, xref_pos: u64, file_len: u64) -> Result<Xref, String> {
    let mut sections: Vec<(HashMap<u32, Entry>, Option<(u32, u16)>)> = Vec::new();
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut pos = xref_pos;
    loop {
        if !visited.insert(pos) || sections.len() > 16 {
            break;
        }
        let head = read_chunk(f, pos, 64)?;
        if head.len() < 4 {
            break;
        }
        if &head[..4] == b"xref" {
            let (t, root, prev) = parse_classic_xref(f, pos, file_len)?;
            sections.push((t, root));
            match prev {
                Some(p) if p >= 0 && (p as u64) < file_len => pos = p as u64,
                _ => break,
            }
        } else {
            let (t, root, prev) = parse_xref_stream(f, pos, file_len)?;
            sections.push((t, root));
            match prev {
                Some(p) if p >= 0 && (p as u64) < file_len => pos = p as u64,
                _ => break,
            }
        }
    }
    if sections.is_empty() {
        return Err("无有效 xref".into());
    }
    let mut entries: HashMap<u32, Entry> = HashMap::new();
    let mut root = None;
    for (t, r) in sections.iter().rev() {
        for (k, v) in t {
            entries.insert(*k, *v);
        }
        if root.is_none() {
            root = *r;
        }
    }
    if entries.is_empty() {
        return Err("空 xref".into());
    }
    Ok(Xref { entries, root })
}
