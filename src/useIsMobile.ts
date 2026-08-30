import { useEffect, useState } from "react";

/**
 * 是否手机端视口（宽度 ≤ breakpoint）。
 * 断点与 App.css 的 @media (max-width: 768px) 保持一致，
 * 供 JS 端决定渲染 PC 侧边栏布局还是手机端壳布局。
 */
export function useIsMobile(breakpoint = 768): boolean {
  const [isMobile, setIsMobile] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.matchMedia(`(max-width: ${breakpoint}px)`).matches;
  });

  useEffect(() => {
    const mq = window.matchMedia(`(max-width: ${breakpoint}px)`);
    const onChange = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [breakpoint]);

  return isMobile;
}
