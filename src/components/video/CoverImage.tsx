import { useEffect, useState } from "react";
import { api } from "../../api";
import { ensureVideoCover } from "./videoMedia";

interface Props {
  videoId: string;
  version?: number;
  className?: string;
  style?: React.CSSProperties;
}

// 全局封面加载并发限制（避免大量视频同时触发截帧/网络请求）
const MAX_CONCURRENT = 4;
let inFlight = 0;
const waitQueue: Array<() => void> = [];

function acquire(): Promise<void> {
  return new Promise((resolve) => {
    if (inFlight < MAX_CONCURRENT) {
      inFlight++;
      resolve();
    } else {
      waitQueue.push(() => {
        inFlight++;
        resolve();
      });
    }
  });
}

function release() {
  inFlight--;
  waitQueue.shift()?.();
}

/** 视频封面（按需 JS 截帧并缓存，version 变化时强制刷新） */
export default function CoverImage({ videoId, version = 0, className, style }: Props) {
  const [src, setSrc] = useState<string>();

  useEffect(() => {
    let cancelled = false;
    setSrc(undefined);
    (async () => {
      await acquire();
      try {
        if (cancelled) return;
        // 无现成封面（桌面走失败信号，浏览器走 has_video_cover）→ JS 截帧生成
        let has = true;
        try {
          has = await api.hasVideoCover(videoId);
        } catch {
          has = true; // 命令不可用时按有封面处理，避免误截
        }
        if (!has) {
          const video = await api.getVideo(videoId);
          if (!video || !video.path) throw new Error("no-cover");
          const ok = await ensureVideoCover(video);
          if (!ok) throw new Error("no-cover");
          if (cancelled) return;
        }
        const url = await api.getVideoCoverDataUrl(videoId);
        if (!cancelled) setSrc(url);
      } catch {
        // 无封面 / 无法截帧时保持占位
      } finally {
        release();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [videoId, version]);

  if (!src) {
    return <div className={className} style={{ ...style, background: "var(--bg-badge)" }} />;
  }
  return <img className={className} style={style} src={src} draggable={false} alt="" />;
}
