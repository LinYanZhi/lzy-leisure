// 视频封面截帧与元数据补全工具（无 ffmpeg 依赖：<video> + canvas）
//
// 方案背景：安装包瘦身，去掉打包的 ffmpeg.exe。
//   - 封面：本地视频经 asset 协议（桌面）/ HTTP Range 流（浏览器）加载到隐藏 <video>，
//     seek 到 30% 处用 canvas 截帧为 jpeg data URL，再走既有 upload_cover 命令保存。
//   - 元数据：<video> 的 loadedmetadata 可拿时长/分辨率（fps 浏览器不暴露，保持空）。
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, isWeb, videoStreamUrl, type Video } from "../../api";

/** 封面截帧目标宽度（封面显示仅需几百 px，宽 480 足够且编码快） */
const MAX_COVER_W = 480;

/** 秒数格式化为 mm:ss / hh:mm:ss（与后端旧 ffmpeg 输出格式一致） */
export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = m < 10 ? `0${m}` : `${m}`;
  const ss = r < 10 ? `0${r}` : `${r}`;
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

/** 隐藏 <video> 加载本地视频（只读文件头元数据，随后可 seek 截帧） */
function loadVideoEl(url: string): Promise<HTMLVideoElement> {
  return new Promise((resolve, reject) => {
    const v = document.createElement("video");
    v.preload = "metadata";
    v.muted = true;
    v.playsInline = true;
    const onError = () => {
      cleanup();
      v.remove();
      reject(new Error("video load error"));
    };
    const onLoaded = () => {
      cleanup();
      resolve(v);
    };
    const cleanup = () => {
      v.removeEventListener("loadedmetadata", onLoaded);
      v.removeEventListener("error", onError);
    };
    v.addEventListener("loadedmetadata", onLoaded, { once: true });
    v.addEventListener("error", onError, { once: true });
    v.src = url;
  });
}

/** seek 到指定秒并截取当前帧为 jpeg data URL */
function grabFrame(v: HTMLVideoElement, secs: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const onSeeked = () => {
      v.removeEventListener("seeked", onSeeked);
      try {
        const vw = v.videoWidth || 1;
        const vh = v.videoHeight || 1;
        const scale = Math.min(1, MAX_COVER_W / vw);
        const w = Math.max(1, Math.round(vw * scale));
        const h = Math.max(1, Math.round(vh * scale));
        const canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;
        const ctx = canvas.getContext("2d");
        if (!ctx) {
          reject(new Error("canvas 不可用"));
          return;
        }
        ctx.drawImage(v, 0, 0, w, h);
        resolve(canvas.toDataURL("image/jpeg", 0.75));
      } catch (e) {
        reject(e);
      }
    };
    v.addEventListener("seeked", onSeeked, { once: true });
    v.currentTime = secs;
  });
}

/** 释放隐藏 video 占用的资源 */
function disposeVideo(v: HTMLVideoElement | null) {
  if (!v) return;
  v.removeAttribute("src");
  try {
    v.load();
  } catch {}
}

// ── 封面截帧 ──

// 会话级去重：同一视频只尝试一次截帧（成功或失败均不重复，避免列表滚动反复触发）
const coverAttempted = new Set<string>();

/**
 * 无封面时用 JS 截帧生成封面并保存（成功返回 true）。
 * force 用于"重新生成封面"：忽略会话去重，强制重新截取。
 */
export async function ensureVideoCover(video: Video, force = false): Promise<boolean> {
  if (!video.path) return false;
  if (!force && coverAttempted.has(video.id)) return false;
  coverAttempted.add(video.id);
  let v: HTMLVideoElement | null = null;
  try {
    const url = isWeb ? videoStreamUrl(video.id) : convertFileSrc(video.path);
    v = await loadVideoEl(url);
    const dur = v.duration && isFinite(v.duration) ? v.duration : 10;
    const target = dur > 3 ? dur * 0.3 : 1;
    const dataUrl = await grabFrame(v, target);
    await api.uploadCover(video.id, dataUrl);
    return true;
  } catch {
    return false;
  } finally {
    disposeVideo(v);
  }
}

// ── 元数据补全 ──

const metaPending = new Map<string, Promise<MediaMetaPatch | null>>();

export interface MediaMetaPatch {
  duration: string;
  frameWidth: number | null;
  frameHeight: number | null;
}

/**
 * 读取视频时长/分辨率（仅对缺元数据的新扫描视频；写回后端，大小与格式由后端读取）。
 * 并发按 video.id 去重；Chromium 解不了的格式返回 null（保持空，与播放能力一致）。
 */
export function probeVideoMeta(video: Video): Promise<MediaMetaPatch | null> {
  if (video.duration) return Promise.resolve(null);
  const hit = metaPending.get(video.id);
  if (hit) return hit;
  const p = doProbe(video).finally(() => metaPending.delete(video.id));
  metaPending.set(video.id, p);
  return p;
}

async function doProbe(video: Video): Promise<MediaMetaPatch | null> {
  if (!video.path) return null;
  let v: HTMLVideoElement | null = null;
  try {
    const url = isWeb ? videoStreamUrl(video.id) : convertFileSrc(video.path);
    v = await loadVideoEl(url);
    const dur = v.duration && isFinite(v.duration) ? v.duration : 0;
    const w = v.videoWidth || null;
    const h = v.videoHeight || null;
    const patch: MediaMetaPatch = {
      duration: dur > 0 ? fmtDuration(dur) : "",
      frameWidth: w,
      frameHeight: h,
    };
    await api.updateVideoMediaMeta(video.id, patch.duration, w, h);
    return patch;
  } catch {
    return null;
  } finally {
    disposeVideo(v);
  }
}
