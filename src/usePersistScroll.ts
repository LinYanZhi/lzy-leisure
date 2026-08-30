// ============================================================
// usePersistScroll — 列表页滚动位置记忆
//
// 内容就绪后恢复上次滚动位置；滚动时防抖写入；卸载时立即保存最后位置。
// 数据存放在 DataManager 的 "comic-scroll"（JSON map，键 → scrollTop）。
// 键由调用方指定：书架用 shelf:<root>（按导入路径记忆）、章节列表用 series:<id>。
// ============================================================

import { useEffect, useMemo, useRef } from "react";
import { useData } from "@glbt/appkit-ui";
import { store } from "./data";

const FLUSH_DELAY = 200;

type ScrollMap = Record<string, number>;

function parseMap(raw: string): ScrollMap {
  try {
    const v = JSON.parse(raw);
    if (v && typeof v === "object" && !Array.isArray(v)) return v as ScrollMap;
  } catch {
    /* 损坏数据忽略 */
  }
  return {};
}

export function usePersistScroll(
  key: string,
  elRef: React.RefObject<HTMLElement | null>,
  ready: boolean,
): void {
  const [raw, setRaw] = useData(store.keys["comic-scroll"]);
  const map = useMemo(() => parseMap(raw), [raw]);
  // 已按某键完成过恢复（键切换或重新挂载后重新恢复）
  const restoredKeyRef = useRef<string | null>(null);

  useEffect(() => {
    if (!ready) return;
    const el = elRef.current;
    if (!el) return;
    // 内容就绪后恢复滚动位置（键变化或重新挂载时恢复一次）
    if (restoredKeyRef.current !== key) {
      const saved = map[key] ?? 0;
      if (saved > 0) el.scrollTop = saved;
      restoredKeyRef.current = key;
    }
    // 滚动防抖写入；卸载时立即保存最后位置
    let timer: ReturnType<typeof setTimeout> | null = null;
    const save = () => {
      if (timer) clearTimeout(timer);
      timer = null;
      const top = el.scrollTop;
      setRaw((prev) => {
        const m = parseMap(prev);
        if (m[key] === top) return prev;
        return JSON.stringify({ ...m, [key]: top });
      });
    };
    const onScroll = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(save, FLUSH_DELAY);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      if (timer) save(); // 卸载/重挂载前把最后一次滚动落盘
      el.removeEventListener("scroll", onScroll);
    };
  }, [ready, key, map, elRef, setRaw]);
}
