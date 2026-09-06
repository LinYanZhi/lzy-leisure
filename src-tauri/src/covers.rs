//! 漫画封面生成与缓存
//!
//! 封面优先取 cover.* 文件，其次取第一页，裁剪为 5:7 缩略图（jpeg）后
//! 存入漫画本体目录的外部封面库（.leisure-covers.db，唯一来源）。
//! 对极端长图（如长条漫画）取顶部区域，避免居中裁到无意义的中段。
//! 自定义封面：章节多页垂直拼接成条带，按裁剪偏移截取 5:7 窗口生成（对齐旧项目 MangaShelf）。
use crate::db::{self, comics::{Comic, CoverData}};
use crate::scanner;
use image::codecs::jpeg::JpegEncoder;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 封面目标尺寸（5:7 比例，统一 standard 一档）
pub const COVER_SIZE: (u32, u32) = (400, 560);

/// 高于该比例视为"极端长图"，取顶部
const EXTREME_RATIO_THRESHOLD: f64 = 3.0;

/// 生成 5:7 缩略图并编码为 jpeg，返回 (字节, mime, 宽, 高)
pub(crate) fn generate_thumbnail(data: &[u8], target: (u32, u32)) -> Result<(Vec<u8>, String, u32, u32), String> {
    let img = image::load_from_memory(data).map_err(|e| format!("解码图片失败: {e}"))?;

    let target_ratio = target.0 as f64 / target.1 as f64;
    let img_ratio = img.width() as f64 / img.height() as f64;

    let cropped = if img_ratio > target_ratio {
        // 图片更宽 → 横向居中裁剪
        let crop_w = ((img.height() as f64) * target_ratio).round() as u32;
        let offset = (img.width() - crop_w) / 2;
        img.crop_imm(offset, 0, crop_w, img.height())
    } else if img_ratio < target_ratio {
        // 图片更高 → 纵向裁剪（极端长图取顶部）
        let crop_h = ((img.width() as f64) / target_ratio).round() as u32;
        let ratio_diff = target_ratio / img_ratio;
        let offset = if ratio_diff > EXTREME_RATIO_THRESHOLD {
            0
        } else {
            (img.height() - crop_h) / 2
        };
        img.crop_imm(0, offset, img.width(), crop_h)
    } else {
        img
    };

    let resized = cropped.resize_exact(target.0, target.1, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();
    let (w, h) = rgb.dimensions();

    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 85);
    encoder
        .encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("jpeg 编码失败: {e}"))?;

    Ok((out, "image/jpeg".to_string(), w, h))
}

/// 页面条带索引（外部封面库缓存优先，未命中则构建并保存）——供手动封面恢复用
fn cached_page_index(comic: &Comic) -> Result<PageIndex, String> {
    if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
        let key = crate::covers_db::rel_key_of(comic, &bucket);
        if let Some(cached) = crate::covers_db::load_page_index(&bucket, &key)? {
            if let Ok(idx) = serde_json::from_str::<PageIndex>(&cached) {
                if idx.pw == PREVIEW_WIDTH {
                    return Ok(idx);
                }
            }
        }
        let idx = build_page_index(comic)?;
        let data = serde_json::to_string(&idx).map_err(|e| format!("序列化失败: {e}"))?;
        let _ = crate::covers_db::save_page_index(&bucket, &key, &data);
        return Ok(idx);
    }
    build_page_index(comic)
}

/// 有手动裁剪参数（cover_crop_offset）时，用该偏移重新生成手动封面并缓存。
/// 返回 true 表示已生成并写库（调用方直接返回结果即可）。
fn regenerate_manual_cover(
    comic: &Comic,
    bucket: &str,
    key: &str,
    configs: &std::collections::HashMap<String, String>,
) -> Option<CoverData> {
    let offset_str = configs.get("cover_crop_offset")?;
    let offset = offset_str.trim().parse::<u32>().ok()?;
    let idx = cached_page_index(comic).ok()?;
    let (data, mime, w, h) = generate_cropped_cover(comic, &idx, offset, COVER_SIZE).ok()?;
    let _ = crate::covers_db::save_cover(bucket, key, &data, &mime, w as i64, h as i64);
    // 标记已按偏移生成，避免每次请求重复重建
    let mut cfg = configs.clone();
    cfg.insert("cover_from_offset".to_string(), "1".to_string());
    let _ = crate::covers_db::save_configs(bucket, key, &cfg);
    Some(CoverData {
        data,
        mime,
        width: w as i64,
        height: h as i64,
    })
}

/// 获取封面（漫画本体外部封面库唯一来源，其次 cover.* 文件 / 第一页）。
/// series 无自身封面时回退到第一章封面。
pub fn get_cover(comic: &Comic) -> Result<CoverData, String> {
    // 1. 本体外部封面库（唯一来源），key 为相对本体目录的路径
    if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
        let key = crate::covers_db::rel_key_of(comic, &bucket);
        let configs = crate::covers_db::load_configs(&bucket, &key).unwrap_or_default();
        let has_offset = configs.contains_key("cover_crop_offset");
        let already_offset = configs.get("cover_from_offset").map(|s| s.as_str()) == Some("1");
        if let Ok(Some((data, mime, w, h))) = crate::covers_db::load_cover(&bucket, &key) {
            // 无手动裁剪参数，或已按偏移生成过 → 直接用库里的封面
            if !has_offset || already_offset {
                return Ok(CoverData {
                    data,
                    mime,
                    width: w,
                    height: h,
                });
            }
            // 有手动裁剪参数但库里的还是旧默认图（如迁移导入的 webp）→ 用偏移重新生成
            if let Some(cover) = regenerate_manual_cover(comic, &bucket, &key, &configs) {
                return Ok(cover);
            }
            // 重新生成失败也返回现有封面，避免空手
            return Ok(CoverData {
                data,
                mime,
                width: w,
                height: h,
            });
        }
        // 1b. 封面缺失但保存过手动裁剪参数（cover_crop_offset）→ 用该偏移重新生成
        // 手动封面，绝不回退成默认第一页图（手动设置几百章的封面不能被扫描/异常重置）。
        if let Some(cover) = regenerate_manual_cover(comic, &bucket, &key, &configs) {
            return Ok(cover);
        }
    }

    let target = COVER_SIZE;

    // 2. cover.* 文件
    let mut source: Option<(Vec<u8>, String)> = None;
    if !comic.cover_path.is_empty() {
        let p = Path::new(&comic.root_dir).join(&comic.path).join(&comic.cover_path);
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(&p) {
                source = Some((bytes, scanner::mime_for(&comic.cover_path)));
            }
        }
    }

    // 3. 第一页
    if source.is_none() {
        if let Some((bytes, mime)) = scanner::read_page(comic, 0) {
            source = Some((bytes, mime));
        }
    }

    // 4. series 无封面 → 回退第一章封面
    if source.is_none() && comic.kind == "series" {
        let chapters = db::comics::load_chapters(&comic.id)?;
        if let Some(first) = chapters.first().cloned() {
            return get_cover(&first);
        }
    }

    let (bytes, src_mime) = source.ok_or_else(|| "无法获取封面源图".to_string())?;

    match generate_thumbnail(&bytes, target) {
        Ok((data, mime, w, h)) => {
            // 生成结果写回本体外部封面库（唯一来源），key 为相对本体目录的路径
            if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
                let key = crate::covers_db::rel_key_of(comic, &bucket);
                let _ = crate::covers_db::save_cover(
                    &bucket,
                    &key,
                    &data,
                    &mime,
                    w as i64,
                    h as i64,
                );
            }
            Ok(CoverData { data, mime, width: w as i64, height: h as i64 })
        }
        Err(_) => Ok(CoverData { data: bytes, mime: src_mime, width: 0, height: 0 }),
    }
}

// ══════════════════════════════════════════════════════════
//  自定义封面：页面条带索引 + 拼接裁剪
// ══════════════════════════════════════════════════════════

/// 裁剪预览宽度（页面索引以此为基准缩放）
pub const PREVIEW_WIDTH: u32 = 400;

/// 单页尺寸与预览高度（ph = 缩到 PREVIEW_WIDTH 宽后的高度）
#[derive(Serialize, Deserialize, Clone)]
pub struct PageDim {
    pub w: u32,
    pub h: u32,
    pub ph: u32,
}

/// 章节页面条带索引（所有页垂直排列的总览）
#[derive(Serialize, Deserialize, Clone)]
pub struct PageIndex {
    pub pw: u32,
    pub pages: Vec<PageDim>,
    pub total_ph: u32,
}

/// 从读取器解析像素尺寸（仅读文件头，不解码全图）
fn read_dimensions_from<R: std::io::BufRead + std::io::Seek>(reader: R) -> Option<(u32, u32)> {
    let reader = ImageReader::new(reader);
    let reader = reader.with_guessed_format().ok()?;
    reader.into_dimensions().ok()
}

/// 构建章节页面条带索引（失败页跳过，流式读头避免整读大文件）
pub fn build_page_index(comic: &Comic) -> Result<PageIndex, String> {
    let names = scanner::list_page_files(comic);
    let pw = PREVIEW_WIDTH;
    let mut pages = Vec::new();
    let mut total_ph: u64 = 0;
    for name in &names {
        let dims = match comic.kind.as_str() {
            "folder" => {
                let p = Path::new(&comic.root_dir).join(&comic.path).join(name);
                std::fs::File::open(&p)
                    .ok()
                    .and_then(|f| read_dimensions_from(std::io::BufReader::new(f)))
            }
            "archive" => {
                let full_path = Path::new(&comic.root_dir).join(&comic.path);
                std::fs::File::open(&full_path)
                    .ok()
                    .and_then(|f| zip::ZipArchive::new(f).ok())
                    .and_then(|mut z| {
                        z.by_name(name).ok().and_then(|mut e| {
                            // 只需文件头部即可解析像素尺寸，避免整页解压大图
                            let mut head = Vec::new();
                            std::io::Read::read_to_end(
                                &mut std::io::Read::take(&mut e, 128 * 1024),
                                &mut head,
                            )
                            .ok()?;
                            read_dimensions_from(std::io::Cursor::new(head))
                        })
                    })
            }
            _ => None,
        };
        if let Some((w, h)) = dims {
            let ph = ((h as u64) * (pw as u64) / (w.max(1) as u64)).max(1) as u32;
            total_ph += ph as u64;
            pages.push(PageDim { w, h, ph });
        }
    }
    if pages.is_empty() {
        return Err("无法分析章节页面".to_string());
    }
    Ok(PageIndex {
        pw,
        pages,
        total_ph: total_ph as u32,
    })
}

/// 按指定宽度缩放图片（保持比例，jpeg 编码），供条带预览使用
pub fn resize_to_width(data: &[u8], width: u32) -> Option<(Vec<u8>, String, u32, u32)> {
    let img = image::load_from_memory(data).ok()?;
    let w = img.width();
    let h = img.height();
    let scale = width as f64 / w.max(1) as f64;
    let target_h = ((h as f64) * scale).round().max(1.0) as u32;
    let resized = img.resize(width, target_h, image::imageops::FilterType::Triangle);
    let rgb = resized.to_rgb8();
    let (rw, rh) = rgb.dimensions();
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 80);
    encoder
        .encode(rgb.as_raw(), rw, rh, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some((out, "image/jpeg".to_string(), rw, rh))
}

/// 定位预览条带上某像素偏移所在的页 (page_idx, 页内局部偏移)
fn find_page_at_offset(pages: &[PageDim], offset: u32) -> (usize, u32) {
    let mut cum = 0u32;
    for (i, p) in pages.iter().enumerate() {
        if offset < cum + p.ph {
            return (i, offset.saturating_sub(cum));
        }
        cum += p.ph;
    }
    let last = pages.len().saturating_sub(1);
    (last, pages[last].ph)
}

/// 获取某页 400 宽预览图（外部封面库缓存优先，未命中则生成并缓存）。
/// 预览图与裁剪窗口同为 PREVIEW_WIDTH 基准，可 1:1 拼接裁剪封面。
fn preview_image(comic: &Comic, page_idx: usize) -> Result<image::DynamicImage, String> {
    if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
        let key = crate::covers_db::rel_key_of(comic, &bucket);
        if let Some((data, _mime)) =
            crate::covers_db::load_preview(&bucket, &key, page_idx)?
        {
            if let Ok(img) = image::load_from_memory(&data) {
                return Ok(img);
            }
        }
        let (bytes, _) = scanner::read_page(comic, page_idx)
            .ok_or_else(|| format!("页面 {page_idx} 读取失败"))?;
        let (data, mime, w, h) = resize_to_width(&bytes, PREVIEW_WIDTH)
            .ok_or_else(|| format!("页面 {page_idx} 预览生成失败"))?;
        let _ = crate::covers_db::save_preview(
            &bucket,
            &key,
            page_idx,
            &data,
            &mime,
            w as i64,
            h as i64,
        );
        return image::load_from_memory(&data).map_err(|e| format!("预览解码失败: {e}"));
    }
    Err("无法定位漫画本体".to_string())
}

/// 拼接裁剪封面：从 400px 预览条带截取 [offset, offset+窗口高) 区域生成 5:7 封面。
/// 所见即所得（与弹窗裁剪窗口显示一致），预览图已缓存时几乎纯像素操作、速度极快。
pub fn generate_cropped_cover(
    comic: &Comic,
    index: &PageIndex,
    offset: u32,
    target: (u32, u32),
) -> Result<(Vec<u8>, String, u32, u32), String> {
    let pw = index.pw.max(1);
    let ch = (pw as u64 * 7 / 5).max(1) as u32; // 预览坐标下的窗口高度
    let total_ph = index.total_ph;
    if index.pages.is_empty() || total_ph == 0 {
        return Err("章节无页面".to_string());
    }

    let max_off = total_ph.saturating_sub(ch);
    let offset = offset.min(max_off);

    // 逐页从预览图取覆盖 [offset, offset+ch) 的条带
    let mut strips: Vec<image::DynamicImage> = Vec::new();
    let mut remaining = ch as i64;
    let mut cur = offset;
    while remaining > 0 && (cur as u64) < total_ph as u64 {
        let (page_idx, local_off) = find_page_at_offset(&index.pages, cur);
        let page = &index.pages[page_idx];
        let take_preview = (page.ph.saturating_sub(local_off)).min(remaining as u32);
        if take_preview == 0 {
            cur = cur.saturating_add(1);
            continue;
        }
        let img = match preview_image(comic, page_idx) {
            Ok(img) => img,
            // 单页失败则跳过该页区域（保留缺口，其余页照常拼接）
            Err(_) => {
                remaining -= take_preview as i64;
                cur += take_preview;
                continue;
            }
        };
        let crop_y = local_off.min(img.height().saturating_sub(1));
        let crop_h = take_preview.min(img.height().saturating_sub(crop_y));
        if crop_h == 0 {
            break;
        }
        if crop_h >= img.height() {
            strips.push(img);
        } else {
            strips.push(img.crop_imm(0, crop_y, img.width(), crop_h));
        }
        remaining -= crop_h as i64;
        cur += crop_h;
    }

    if strips.is_empty() {
        return Err("页面拼接失败".to_string());
    }

    // 垂直拼接（宽度统一 PREVIEW_WIDTH）
    let full_h: u32 = strips.iter().map(|s| s.height()).sum();
    let mut stitched = image::RgbImage::new(pw, full_h);
    {
        let mut y = 0u32;
        for s in &strips {
            let rgb = s.to_rgb8();
            for (dx, dy, px) in rgb.enumerate_pixels() {
                stitched.put_pixel(dx, y + dy, *px);
            }
            y += s.height();
        }
    }

    // 补足到目标高度（底部不足时黑底填充），再缩放到目标尺寸
    let mut canvas = image::DynamicImage::ImageRgb8(stitched);
    if canvas.height() < target.1 {
        let mut padded = image::RgbImage::from_pixel(target.0, target.1, image::Rgb([0, 0, 0]));
        let offset_y = (target.1 - canvas.height()) / 2;
        let rgb = canvas.to_rgb8();
        for (dx, dy, px) in rgb.enumerate_pixels() {
            let py = offset_y + dy;
            if py < target.1 && dx < target.0 {
                padded.put_pixel(dx, py, *px);
            }
        }
        canvas = image::DynamicImage::ImageRgb8(padded);
    }
    let resized = canvas.resize_exact(target.0, target.1, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();
    let (w, h) = rgb.dimensions();

    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 85);
    encoder
        .encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("jpeg 编码失败: {e}"))?;

    Ok((out, "image/jpeg".to_string(), w, h))
}
