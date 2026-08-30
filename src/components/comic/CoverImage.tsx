import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../api";

interface Props {
  comicId: string;
  className?: string;
  style?: React.CSSProperties;
}

// 全局封面加载并发限制（避免大量章节同时触发后端封面生成，防止 UI 卡顿）
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

/** 漫画封面（进入视口才加载，带全局并发限制；后端生成不阻塞 UI 线程） */
export default function CoverImage({ comicId, className, style }: Props) {
  const elRef = useRef<HTMLElement | null>(null);
  const setEl = useCallback((el: HTMLElement | null) => {
    elRef.current = el;
  }, []);
  const [src, setSrc] = useState<string>();

  useEffect(() => {
    let cancelled = false;
    const el = elRef.current;
    if (!el) return;

    const load = async () => {
      await acquire();
      try {
        if (cancelled) return;
        const url = await api.getComicCoverDataUrl(comicId);
        if (!cancelled) setSrc(url);
      } catch {
        // 封面不可用时保持占位
      } finally {
        release();
      }
    };

    // 进入视口才加载（章节列表卡片很多，避免一次性并发大量请求）
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            io.disconnect();
            load();
            break;
          }
        }
      },
      { rootMargin: "400px 0px" },
    );
    io.observe(el);
    return () => {
      cancelled = true;
      io.disconnect();
    };
  }, [comicId]);

  if (!src) {
    return <div ref={setEl} className={className} style={{ ...style, background: "var(--bg-badge)" }} />;
  }
  return <img ref={setEl} className={className} style={style} src={src} draggable={false} alt="" />;
}
