//! 故事会 PDF 第一页封面图按需读取（seek 式，不整读 PDF）
//!
//! `lopdf::Document::load` 会把整个 PDF 读入内存再解析全部对象；大扫描件
//! （几十 MB/期）只为取第一页封面图代价过高。本模块只做最小解析：
//! 文件尾 startxref → xref（经典表 / xref 流 / 对象流）→
//! Root → Pages → 第一页 → Resources → XObject → 第一个 DCTDecode 图片流，
//! 期间只用 seek 读取用到的字节段。任何一步失败返回 Err，调用方回退 lopdf 整读。
//!
//! 解析器按层拆分为子模块（外部 API 不变）：
//!   - `pdf_objects.rs` 对象层：极简对象模型（Obj/Parser/Reader）与字节解析
//!   - `pdf_xref.rs`    xref 层：startxref / 经典表 / xref 流 / /Prev 链组装
//!   - 本文件保留应用层：第一页 JPEG 提取、位图转 JPEG、页数统计
mod pdf_objects;
mod pdf_xref;

use pdf_objects::{decode_parms, inflate, png_undo_predictor, read_chunk, Obj, Reader};
use pdf_xref::{build_xref, find_startxref, Xref};
use std::fs::File;
use std::path::Path;

/// 取页面树节点的 Kids 数组：兼容内联 `/Kids [..]` 与间接引用 `/Kids 4 0 R`（对象 4 为数组）两种写法
fn node_kids(r: &mut Reader, node: &Obj) -> Result<Vec<Obj>, String> {
    match node.dict_get(b"Kids") {
        Some(Obj::Array(a)) => Ok(a.clone()),
        Some(Obj::Ref(n, _)) => match r.get_object(*n)? {
            Obj::Array(a) => Ok(a),
            _ => Err("Kids 引用非数组".into()),
        },
        _ => Err("页面树节点缺 Kids".into()),
    }
}

/// 子树页数（递归；页面树中间节点往下数的叶页面总数）
fn subtree_page_count(r: &mut Reader, node_num: u32, depth: usize) -> Result<usize, String> {
    if depth > 64 {
        return Err("页面树过深".into());
    }
    let node = r.get_object(node_num)?;
    let kids = node_kids(r, &node)?;
    let mut total = 0usize;
    for kid in kids {
        let Obj::Ref(num, _) = kid else { continue };
        let kobj = r.get_object(num)?;
        if kobj.dict_get(b"Kids").is_some() {
            total += subtree_page_count(r, num, depth + 1)?;
        } else {
            total += 1;
        }
    }
    Ok(total)
}

/// 页面树中取第 idx 个（0 起）叶页面对象号（深度优先，按文档顺序）
fn nth_page_ref(r: &mut Reader, pages_num: u32, idx: usize, depth: usize) -> Result<u32, String> {
    if depth > 32 {
        return Err("页面树过深".into());
    }
    let obj = r.get_object(pages_num)?;
    let kids = node_kids(r, &obj)?;
    let mut remain = idx;
    for kid in kids {
        let Obj::Ref(num, _) = kid else { continue };
        let kobj = r.get_object(num)?;
        if kobj.dict_get(b"Kids").is_some() {
            let cnt = subtree_page_count(r, num, 0)?;
            if remain >= cnt {
                remain -= cnt;
                continue;
            }
            return nth_page_ref(r, num, remain, depth + 1);
        }
        if remain == 0 {
            return Ok(num);
        }
        remain -= 1;
    }
    Err("页码越界".into())
}

/// 清除 JPEG 流前导（EOL / PDF 允许的 EOI 标记）
fn strip_jpeg_prefix(b: &[u8]) -> Vec<u8> {
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i + 1 < b.len() && b[i] == 0xFF && b[i + 1] == 0xD9 {
        i += 2;
    }
    b[i..].to_vec()
}

/// 廉价 JPEG 结构校验：SOI（FF D8）开头 + 文件尾附近 EOI（FF D9）。
/// 不做整图解码——扫描件第一页 JPEG 全图解码可耗时数百 ms（且 downscale 还会再解一次）。
fn is_plausible_jpeg(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i + 1 >= b.len() || b[i] != 0xFF || b[i + 1] != 0xD8 {
        return false;
    }
    let scan_start = b.len().saturating_sub(256);
    b[scan_start..].windows(2).any(|w| w == [0xFF, 0xD9])
}

/// 打开 PDF 并解析 xref（seek 式：只读文件尾 + xref 区段，不整读文件）。
/// 任何一步失败返回 Err，调用方回退 lopdf。
fn open_pdf(path: &Path) -> Result<(File, Xref), String> {
    let mut f = File::open(path).map_err(|e| format!("打开 PDF 失败: {e}"))?;
    let file_len = f.metadata().map_err(|e| e.to_string())?.len();
    if file_len < 64 {
        return Err("文件过小".into());
    }
    // startxref（文件尾，规范要求在最后 1024 字节内，放宽到 8KB 兜底）
    let tail_len = (8192usize).min(file_len as usize);
    let tail = read_chunk(&mut f, file_len - tail_len as u64, tail_len)?;
    let startxref = find_startxref(&tail)?;
    if startxref < 0 || startxref as u64 >= file_len {
        return Err("startxref 越界".into());
    }
    // xref：经典表 / xref 流，含增量更新 /Prev 链
    let xref = build_xref(&mut f, startxref as u64, file_len)?;
    Ok((f, xref))
}

/// 从单个页面对象提取第一个可解码整页 JPEG（DCTDecode）流字节
/// （Resources → XObject → 第一个可解码 DCTDecode 图；FlateDecode 位图兜底）
fn page_jpeg_from_page(r: &mut Reader, page: &Obj) -> Result<Vec<u8>, String> {
    let resources = page.dict_get(b"Resources").cloned().ok_or("页面缺 Resources")?;
    let resources = match resources {
        Obj::Ref(n, _) => r.get_object(n)?,
        other => other,
    };
    let xobj = resources.dict_get(b"XObject").cloned().ok_or("缺 XObject")?;
    let xobj = match xobj {
        Obj::Ref(n, _) => r.get_object(n)?,
        other => other,
    };
    let xobj_dict = xobj.as_dict().ok_or("XObject 非字典")?;
    // FlateDecode 位图兜底：取内容最大的一张（避免误选 SMask 掩码等小图）
    let mut bitmap_fallback: Option<Vec<u8>> = None;
    for (_name, v) in xobj_dict {
        let img = match v {
            Obj::Ref(n, _) => r.get_object(*n)?,
            other => other.clone(),
        };
        let (img_dict, data) = match img {
            Obj::Stream { dict, data } => (dict, data),
            _ => continue,
        };
        let is_image = img_dict
            .iter()
            .any(|(k, val)| k == b"Subtype" && val.as_name() == Some(b"Image"));
        if !is_image {
            continue;
        }
        let filter = img_dict.iter().find(|(k, _)| k == b"Filter");
        let is_jpeg = match filter {
            Some((_, Obj::Name(n))) => n == b"DCTDecode",
            Some((_, Obj::Array(a))) => a.iter().any(|o| o.as_name() == Some(b"DCTDecode")),
            _ => false,
        };
        if is_jpeg {
            let bytes = strip_jpeg_prefix(&data);
            if is_plausible_jpeg(&bytes) {
                return Ok(bytes);
            }
            // 该图结构异常（非 JPEG 流）→ 试下一个 DCTDecode 图
            continue;
        }
        // FlateDecode 位图（电子排版版封面常为未压缩位图）：组装为 JPEG 后作为兜底
        if let Ok(jpeg) = bitmap_to_jpeg(&img_dict, &data) {
            if bitmap_fallback.as_ref().map(|b| b.len()).unwrap_or(0) < jpeg.len() {
                bitmap_fallback = Some(jpeg);
            }
        }
    }
    if let Some(j) = bitmap_fallback {
        return Ok(j);
    }
    Err("该页无整页 JPEG 图片".into())
}

/// 按需提取第 idx 页（0 起）的第一个可解码整页 JPEG（DCTDecode）流字节。
/// 只读 startxref + xref + 目标页图片流；任何一步失败返回 Err（调用方回退 lopdf）。
pub(crate) fn page_jpeg_stream(path: &Path, idx: usize) -> Result<Vec<u8>, String> {
    let (mut f, xref) = open_pdf(path)?;
    let root = xref.root.ok_or("缺少根对象")?;
    let mut r = Reader { f: &mut f, xref: &xref };

    // Root → Pages → 目标页 → Resources → XObject → 第一个可解码 DCTDecode 图
    let catalog = r.get_object(root.0)?;
    let pages_ref = catalog
        .dict_get(b"Pages")
        .and_then(|v| v.as_ref())
        .ok_or("目录缺 Pages")?;
    let page_num = nth_page_ref(&mut r, pages_ref.0, idx, 0)?;
    let page = r.get_object(page_num)?;
    page_jpeg_from_page(&mut r, &page)
}

/// FlateDecode 位图 → JPEG：解压 + PNG 逆预测 + 按颜色空间组装。
/// 仅支持 8bit 的 DeviceRGB / DeviceGray / DeviceCMYK；其余返回 Err（调用方跳过）。
fn bitmap_to_jpeg(dict: &[(Vec<u8>, Obj)], data: &[u8]) -> Result<Vec<u8>, String> {
    let get = |key: &[u8]| dict.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let is_flate = match get(b"Filter") {
        Some(Obj::Name(n)) => n == b"FlateDecode",
        Some(Obj::Array(a)) => a.iter().any(|o| o.as_name() == Some(b"FlateDecode")),
        _ => false,
    };
    if !is_flate {
        return Err("非 FlateDecode 位图".into());
    }
    let w = get(b"Width").and_then(|v| v.as_int()).ok_or("位图缺 Width")? as u32;
    let h = get(b"Height").and_then(|v| v.as_int()).ok_or("位图缺 Height")? as u32;
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err("位图尺寸异常".into());
    }
    let bpc = get(b"BitsPerComponent").and_then(|v| v.as_int()).unwrap_or(8);
    if bpc != 8 {
        return Err("位图非 8bit".into());
    }
    let channels = match get(b"ColorSpace").map(|v| v.as_name()) {
        Some(Some(b"DeviceRGB")) => 3,
        Some(Some(b"DeviceGray")) => 1,
        Some(Some(b"DeviceCMYK")) => 4,
        _ => return Err("位图颜色空间不支持".into()),
    };
    let mut bytes = inflate(data)?;
    // PNG 预测器（Predictor 10-15）：行数据带每行 1 字节 filter 前缀，
    // Columns 为每行像素字节数；TIFF 预测（Predictor 2）与无预测无需处理
    let (predictor, dp_cols) = decode_parms(dict);
    let columns = if dp_cols > 1 { dp_cols } else { w as usize * channels };
    if (10..=15).contains(&predictor) {
        bytes = png_undo_predictor(&bytes, columns)?;
    }
    let need = w as usize * h as usize * channels;
    if bytes.len() < need {
        return Err("位图数据不足".into());
    }
    let img = match channels {
        1 => {
            let gray = image::GrayImage::from_raw(w, h, bytes[..need].to_vec())
                .ok_or("灰图构建失败")?;
            image::DynamicImage::ImageLuma8(gray).to_rgb8()
        }
        3 => image::RgbImage::from_raw(w, h, bytes[..need].to_vec())
            .ok_or("RGB 图构建失败")?,
        4 => {
            // CMYK → RGB 近似（K 通道按比例暗化）
            let mut rgb = Vec::with_capacity(need * 3 / 4);
            for px in bytes[..need].chunks_exact(4) {
                let (c, m, y, k) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
                let inv = 255 - k;
                rgb.extend_from_slice(&[
                    ((255 - c) * inv / 255) as u8,
                    ((255 - m) * inv / 255) as u8,
                    ((255 - y) * inv / 255) as u8,
                ]);
            }
            image::RgbImage::from_raw(w, h, rgb).ok_or("CMYK 图构建失败")?
        }
        _ => return Err("通道数异常".into()),
    };
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG 编码失败: {e}"))?;
    Ok(out)
}

/// 递归统计页面树页数（只读各节点字典，不读页面内容流）
fn count_pages_in_tree(r: &mut Reader, node_num: u32, depth: usize, count: &mut u32) -> Result<(), String> {
    if depth > 64 {
        return Err("页面树过深".into());
    }
    let node = r.get_object(node_num)?;
    let kids = node_kids(r, &node)?;
    for kid in kids {
        let Obj::Ref(num, _) = kid else { continue };
        let kobj = r.get_object(num)?;
        if kobj.dict_get(b"Kids").is_some() {
            count_pages_in_tree(r, num, depth + 1, count)?;
        } else {
            *count += 1;
        }
    }
    Ok(())
}

/// 按需统计 PDF 页数（seek 式：只读页面树字典，不整读 PDF、不解析内容流）。
/// 任何一步失败返回 Err，调用方回退 lopdf。
pub(crate) fn page_count(path: &Path) -> Result<i64, String> {
    let (mut f, xref) = open_pdf(path)?;
    let root = xref.root.ok_or("缺少根对象")?;
    let mut r = Reader { f: &mut f, xref: &xref };
    let catalog = r.get_object(root.0)?;
    let pages_ref = catalog
        .dict_get(b"Pages")
        .and_then(|v| v.as_ref())
        .ok_or("目录缺 Pages")?;
    let mut count = 0u32;
    count_pages_in_tree(&mut r, pages_ref.0, 0, &mut count)?;
    if count == 0 {
        return Err("PDF 没有页面".into());
    }
    Ok(count as i64)
}
