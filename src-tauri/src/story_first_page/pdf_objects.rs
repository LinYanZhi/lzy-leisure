//! 故事会 PDF 最小解析器——对象层：极简对象模型（Obj/Parser/Reader）与字节解析。
//! 原 story_first_page.rs 拆分（xref 层见 pdf_xref，应用见 story_first_page）。
use super::pdf_xref::{Entry, Xref};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// 单次对象解析的读取上限（目录/页面/资源字典都很小，1MB 足够）
const CHUNK: usize = 1 << 20;
/// 对象流 / 解压数据上限，防异常文件撑爆内存
pub(crate) const MAX_BLOB: usize = 32 * 1024 * 1024;

/// 极简 PDF 对象（够封面提取用；字符串值解析时直接跳过，置为 Null）
#[derive(Clone)]
pub(crate) enum Obj {
    Null,
    Bool,
    Int(i64),
    Name(Vec<u8>),
    Ref(u32, u16),
    Array(Vec<Obj>),
    Dict(Vec<(Vec<u8>, Obj)>),
    Stream { dict: Vec<(Vec<u8>, Obj)>, data: Vec<u8> },
}

impl Obj {
    pub(crate) fn as_name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    pub(crate) fn as_int(&self) -> Option<i64> {
        match self {
            Obj::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub(crate) fn as_ref(&self) -> Option<(u32, u16)> {
        match self {
            Obj::Ref(n, g) => Some((*n, *g)),
            _ => None,
        }
    }
    pub(crate) fn as_dict(&self) -> Option<&[(Vec<u8>, Obj)]> {
        match self {
            Obj::Dict(d) | Obj::Stream { dict: d, .. } => Some(d),
            _ => None,
        }
    }
    pub(crate) fn dict_get(&self, key: &[u8]) -> Option<&Obj> {
        self.as_dict()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

/// 字节游标解析器
pub(crate) struct Parser<'a> {
    pub(crate) buf: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn skip_ws(&mut self) {
        while self.pos < self.buf.len() && self.buf[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    pub(crate) fn starts_with(&self, s: &[u8]) -> bool {
        self.buf[self.pos..].starts_with(s)
    }
    pub(crate) fn parse_int(&mut self) -> Option<i64> {
        self.skip_ws();
        let mut i = self.pos;
        let mut neg = false;
        if i < self.buf.len() && (self.buf[i] == b'+' || self.buf[i] == b'-') {
            neg = self.buf[i] == b'-';
            i += 1;
        }
        let start = i;
        while i < self.buf.len() && self.buf[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        // 浮点数（字典里用不到）：连小数部分一起消费，避免残留干扰后续解析
        if i < self.buf.len() && self.buf[i] == b'.' {
            i += 1;
            while i < self.buf.len() && self.buf[i].is_ascii_digit() {
                i += 1;
            }
        }
        let mut v: i64 = 0;
        for &b in &self.buf[start..i] {
            if b.is_ascii_digit() {
                v = v.saturating_mul(10).saturating_add((b - b'0') as i64);
            }
        }
        self.pos = i;
        Some(if neg { -v } else { v })
    }
    pub(crate) fn parse_name(&mut self) -> Result<Vec<u8>, String> {
        if self.buf.get(self.pos) != Some(&b'/') {
            return Err("非名称".into());
        }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.buf.len() {
            let b = self.buf[self.pos];
            if b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b) {
                break;
            }
            self.pos += 1;
        }
        Ok(self.buf[start..self.pos].to_vec())
    }
    pub(crate) fn parse_keyword(&mut self) -> Result<Vec<u8>, String> {
        let start = self.pos;
        while self.pos < self.buf.len() {
            let b = self.buf[self.pos];
            if b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b) {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err("空关键字".into());
        }
        Ok(self.buf[start..self.pos].to_vec())
    }
}

pub(crate) fn parse_dict(p: &mut Parser) -> Result<Vec<(Vec<u8>, Obj)>, String> {
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        if p.starts_with(b">>") {
            p.pos += 2;
            break;
        }
        if p.pos >= p.buf.len() {
            return Err("字典未闭合".into());
        }
        let key = if p.starts_with(b"/") {
            p.parse_name()?
        } else {
            p.parse_keyword()?
        };
        let val = parse_value(p)?;
        out.push((key, val));
    }
    Ok(out)
}

fn parse_array(p: &mut Parser) -> Result<Vec<Obj>, String> {
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        if p.starts_with(b"]") {
            p.pos += 1;
            break;
        }
        if p.pos >= p.buf.len() {
            return Err("数组未闭合".into());
        }
        out.push(parse_value(p)?);
    }
    Ok(out)
}

/// 跳过字面量字符串（含转义与嵌套括号）
fn skip_literal_string(p: &mut Parser) {
    p.pos += 1; // (
    let mut depth = 1;
    while p.pos < p.buf.len() && depth > 0 {
        match p.buf[p.pos] {
            b'\\' => p.pos = (p.pos + 2).min(p.buf.len()),
            b'(' => {
                depth += 1;
                p.pos += 1;
            }
            b')' => {
                depth -= 1;
                p.pos += 1;
            }
            _ => p.pos += 1,
        }
    }
}

/// 跳过十六进制字符串
fn skip_hex_string(p: &mut Parser) {
    p.pos += 1; // <
    while p.pos < p.buf.len() && p.buf[p.pos] != b'>' {
        p.pos += 1;
    }
    if p.pos < p.buf.len() {
        p.pos += 1;
    }
}

fn parse_value(p: &mut Parser) -> Result<Obj, String> {
    p.skip_ws();
    if p.starts_with(b"<<") {
        p.pos += 2;
        return Ok(Obj::Dict(parse_dict(p)?));
    }
    if p.starts_with(b"[") {
        p.pos += 1;
        return Ok(Obj::Array(parse_array(p)?));
    }
    if p.starts_with(b"/") {
        return p.parse_name().map(Obj::Name);
    }
    if p.starts_with(b"(") {
        skip_literal_string(p);
        return Ok(Obj::Null); // 字符串值封面路径用不到
    }
    if p.starts_with(b"<") {
        skip_hex_string(p);
        return Ok(Obj::Null);
    }
    if let Some(i) = p.parse_int() {
        // 可能带 "G R" 的间接引用
        let save = p.pos;
        p.skip_ws();
        if let Some(g) = p.parse_int() {
            let save2 = p.pos;
            p.skip_ws();
            if p.starts_with(b"R") {
                p.pos += 1;
                return Ok(Obj::Ref(i as u32, g as u16));
            }
            p.pos = save2;
        }
        p.pos = save;
        return Ok(Obj::Int(i));
    }
    let kw = p.parse_keyword()?;
    match kw.as_slice() {
        b"true" => Ok(Obj::Bool),
        b"false" => Ok(Obj::Bool),
        b"null" => Ok(Obj::Null),
        _ => Err("未知对象".into()),
    }
}

/// 从文件读取字节段
pub(crate) fn read_chunk(f: &mut File, off: u64, len: usize) -> Result<Vec<u8>, String> {
    f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        let n = f.read(&mut buf[got..]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        got += n;
    }
    buf.truncate(got);
    Ok(buf)
}

pub(crate) fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("FlateDecode 解压失败: {e}"))?;
    if out.len() > MAX_BLOB {
        return Err("解压数据过大".into());
    }
    Ok(out)
}

/// PNG 逆预测（DecodeParms Predictor 10-15）。行数据带每行 1 字节 filter 前缀；
/// 输出还原后的行数据（每行 columns 字节），长度 = 行数 × columns。
pub(crate) fn png_undo_predictor(buf: &[u8], columns: usize) -> Result<Vec<u8>, String> {
    if columns == 0 {
        return Err("预测 Columns 为 0".into());
    }
    let row = columns + 1;
    if buf.is_empty() || buf.len() % row != 0 {
        return Err("预测数据长度不整".into());
    }
    let rows = buf.len() / row;
    let mut out = vec![0u8; rows * columns];
    let mut prev = vec![0u8; columns]; // 上一行还原数据（首行视为全 0）
    for r in 0..rows {
        let filt = buf[r * row];
        let line = &buf[r * row + 1..(r + 1) * row];
        for c in 0..columns {
            let raw = line[c];
            let left = if c > 0 { out[r * columns + c - 1] } else { 0 };
            let up = prev[c];
            let val = match filt {
                0 => raw,                                    // None
                1 => raw.wrapping_add(left),                 // Sub
                2 => raw.wrapping_add(up),                   // Up
                3 => raw.wrapping_add((left + up) / 2),      // Average
                4 => {                                       // Paeth
                    let up_left = if c > 0 { prev[c - 1] } else { 0 };
                    let p = left as i32 + up as i32 - up_left as i32;
                    let (pa, pb, pc) = (
                        (p - left as i32).abs(),
                        (p - up as i32).abs(),
                        (p - up_left as i32).abs(),
                    );
                    let pred = if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        up_left
                    };
                    raw.wrapping_add(pred)
                }
                _ => return Err(format!("未知 PNG filter {filt}").into()),
            };
            out[r * columns + c] = val;
            prev[c] = val;
        }
    }
    Ok(out)
}

/// 从流字典取 DecodeParms 的 (Predictor, Columns)，缺省 (1, 1)
pub(crate) fn decode_parms(dict: &[(Vec<u8>, Obj)]) -> (i64, usize) {
    dict.iter()
        .find(|(k, _)| k == b"DecodeParms")
        .and_then(|(_, v)| v.as_dict())
        .map(|dp| {
            let pred = dp
                .iter()
                .find(|(k, _)| k == b"Predictor")
                .and_then(|(_, v)| v.as_int())
                .unwrap_or(1);
            let cols = dp
                .iter()
                .find(|(k, _)| k == b"Columns")
                .and_then(|(_, v)| v.as_int())
                .unwrap_or(1);
            (pred, cols as usize)
        })
        .unwrap_or((1, 1))
}

/// 按需读取对象（xref 链完整后）；`f/xref` 由外部持有
pub(crate) struct Reader<'a> {
    pub(crate) f: &'a mut File,
    pub(crate) xref: &'a Xref,
}

impl Reader<'_> {
    pub(crate) fn get_object(&mut self, num: u32) -> Result<Obj, String> {
        match *self.xref.entries.get(&num).ok_or_else(|| format!("xref 缺少对象 {num}"))? {
            Entry::Offset(off) => self.read_object_at(off),
            Entry::InObjStm(stm, idx) => {
                let (data, n, first) = self.object_stream_data(stm)?;
                let pairs = parse_objstm(&data, n, first)?;
                let (s, e) = pairs
                    .get(idx as usize)
                    .map(|(_, se)| *se)
                    .ok_or("对象流索引越界")?;
                parse_object_slice(&data[s..e])
            }
        }
    }

    /// 解析偏移处的对象（`N G obj <<dict>> [stream ...] endobj`）
    fn read_object_at(&mut self, off: u64) -> Result<Obj, String> {
        let buf = read_chunk(self.f, off, CHUNK)?;
        if buf.len() < 8 {
            return Err("对象区过短".into());
        }
        let mut p = Parser { buf: &buf, pos: 0 };
        p.skip_ws();
        // 跳过 "N G obj" 头
        if p.parse_int().is_none() {
            return Err("对象头损坏".into());
        }
        p.skip_ws();
        if p.parse_int().is_none() {
            return Err("对象头损坏".into());
        }
        p.skip_ws();
        if !p.starts_with(b"obj") {
            return Err("对象头缺 obj".into());
        }
        p.pos += 3;
        p.skip_ws();
        if !p.starts_with(b"<<") {
            // 顶层对象不一定是字典/流：PDF 允许数字/数组/名称等直接值
            //（如 `5 0 obj 383584 endobj`），按普通值解析
            return parse_value(&mut p);
        }
        p.pos += 2;
        let dict = parse_dict(&mut p)?;
        p.skip_ws();
        if p.starts_with(b"stream") {
            p.pos += 6;
            if p.buf.get(p.pos) == Some(&b'\r') {
                p.pos += 1;
            }
            if p.buf.get(p.pos) == Some(&b'\n') {
                p.pos += 1;
            }
            let data_off = off + p.pos as u64;
            let length = match dict.iter().find(|(k, _)| k == b"Length") {
                Some((_, v)) => self.resolve_int(v)?,
                None => -1,
            };
            let data = if length >= 0 {
                let n = (length as usize).min(MAX_BLOB);
                read_chunk(self.f, data_off, n)?
            } else {
                let rest = &buf[p.pos..];
                let end = find_endstream(rest);
                end.map(|e| rest[..e].to_vec())
                    .ok_or_else(|| "流长度未知".to_string())?
            };
            return Ok(Obj::Stream { dict, data });
        }
        Ok(Obj::Dict(dict))
    }

    fn resolve_int(&mut self, o: &Obj) -> Result<i64, String> {
        match o {
            Obj::Int(i) => Ok(*i),
            Obj::Ref(n, _) => match self.get_object(*n)? {
                Obj::Int(i) => Ok(i),
                _ => Err("Length 引用非整数".into()),
            },
            _ => Err("Length 非整数".into()),
        }
    }

    /// 对象流（ObjStm）解码后的字节 + 头部元数据（N 对象数 / First 首个对象偏移）
    fn object_stream_data(&mut self, stm: u32) -> Result<(Vec<u8>, usize, usize), String> {
        let obj = self.get_object(stm)?;
        let (dict, data) = match obj {
            Obj::Stream { dict, data } => (dict, data),
            _ => return Err("对象流非流对象".into()),
        };
        let flate = dict
            .iter()
            .find(|(k, _)| k == b"Filter")
            .map(|(_, v)| match v {
                Obj::Name(n) => n == b"FlateDecode",
                Obj::Array(a) => a.iter().any(|o| o.as_name() == Some(b"FlateDecode")),
                _ => false,
            })
            .unwrap_or(false);
        let (predictor, columns) = decode_parms(&dict);
        let mut data = if flate { inflate(&data)? } else { data };
        if predictor > 1 {
            data = png_undo_predictor(&data, columns)?;
        }
        let n = dict
            .iter()
            .find(|(k, _)| k == b"N")
            .and_then(|(_, v)| v.as_int())
            .unwrap_or(0) as usize;
        let first = dict
            .iter()
            .find(|(k, _)| k == b"First")
            .and_then(|(_, v)| v.as_int())
            .unwrap_or(0) as usize;
        Ok((data, n, first))
    }
}

fn find_endstream(buf: &[u8]) -> Option<usize> {
    buf.windows(9).position(|w| w == b"endstream")
}

/// 对象流内容：N 个 (对象号, 偏移) 对 → 各对象字节区间。
/// 兼容两种格式，offset 均相对 First（PDF 规范）：
///  - 标准：数据头 "N First"，随后是对象对；
///  - 无头（部分扫描工具）：dict 提供 N/First，数据直接从第一个对象的对象号开始。
fn parse_objstm(data: &[u8], n: usize, first: usize) -> Result<Vec<(u32, (usize, usize))>, String> {
    if n == 0 {
        return Err("对象流 N 缺失".into());
    }
    let mut p = Parser { buf: data, pos: 0 };
    p.skip_ws();
    let head = p.parse_int().ok_or("对象流头部损坏")?;
    let mut pairs: Vec<(u32, usize)> = Vec::with_capacity(n);
    if head as usize == n {
        // 标准格式：头部 "N First"
        p.skip_ws();
        p.parse_int().ok_or("对象流头部损坏")?;
        for _ in 0..n {
            p.skip_ws();
            let num = p.parse_int().ok_or("对象流头部损坏")? as u32;
            p.skip_ws();
            let off = p.parse_int().ok_or("对象流头部损坏")? as usize;
            pairs.push((num, first + off));
        }
    } else {
        // 无头格式：已消费第一个对象的对象号，先补读它的偏移
        p.skip_ws();
        let off = p.parse_int().ok_or("对象流头部损坏")? as usize;
        pairs.push((head as u32, first + off));
        for _ in 1..n {
            p.skip_ws();
            let num = p.parse_int().ok_or("对象流头部损坏")? as u32;
            p.skip_ws();
            let off = p.parse_int().ok_or("对象流头部损坏")? as usize;
            pairs.push((num, first + off));
        }
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let s = pairs[i].1;
        let e = if i + 1 < n { pairs[i + 1].1 } else { data.len() };
        out.push((pairs[i].0, (s.min(data.len()), e.min(data.len()))));
    }
    Ok(out)
}

/// 解析对象流内的一段对象字节（可能带 "N G obj" 头）
fn parse_object_slice(buf: &[u8]) -> Result<Obj, String> {
    let mut p = Parser { buf, pos: 0 };
    p.skip_ws();
    if let Some(_) = p.parse_int() {
        let save = p.pos;
        p.skip_ws();
        if let Some(_) = p.parse_int() {
            let save2 = p.pos;
            p.skip_ws();
            if p.starts_with(b"obj") {
                p.pos += 3;
                return parse_value(&mut p);
            }
            p.pos = save2;
        }
        p.pos = save;
    }
    parse_value(&mut p)
}
