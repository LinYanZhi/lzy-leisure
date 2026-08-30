import { useCallback, useEffect, useRef, useState } from "react";
import { useData } from "@glbt/appkit-ui";
import { api, type Novel } from "../../api";
import { store } from "../../data";

interface Props {
  novel: Novel;
  onClose: () => void;
}

/**
 * 小说阅读器（基础版）：
 *  - 章节侧栏 + 正文滚动阅读
 *  - 滚动防抖自动记忆进度（章节 + 章节内字符偏移），进入时续读恢复
 *  - 深色适配，Q 键 / 返回按钮退出
 */
export default function NovelReaderView({ novel, onClose }: Props) {
  // 初始章节：优先从保存进度恢复（novel.chapter 为章节索引字符串）
  const initialIdx =
    novel.chapter !== "" &&
    Number(novel.chapter) >= 0 &&
    Number(novel.chapter) < novel.chapters.length
      ? Number(novel.chapter)
      : 0;
  const [chapterIdx, setChapterIdx] = useState(initialIdx);
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [showToc, setShowToc] = useState(false);
  // 字号（px）：useData 持久化（字符串存储，范围 13-26 由 changeFont 保证）
  const [fontSizeRaw, setFontSizeRaw] = useData(store.keys["novel-font-size"]);
  const fontSize = Math.min(26, Math.max(13, parseInt(fontSizeRaw, 10) || 17));

  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  // ── 进度计算：当前视口顶部对应的字符偏移 ──
  const currentCharOffset = useCallback((): number => {
    const content = contentRef.current;
    const root = scrollRef.current;
    if (!content || !root) return 0;
    const rootRect = root.getBoundingClientRect();
    const x = rootRect.left + Math.min(120, rootRect.width / 2);
    const y = rootRect.top + 8;

    let node: Node | null = null;
    let off = 0;
    const crange = (
      document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null }
    ).caretRangeFromPoint?.(x, y);
    if (crange && content.contains(crange.startContainer)) {
      node = crange.startContainer;
      off = crange.startOffset;
    } else {
      // 兜底：找第一个下边缘进入视口的文本节点
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      let n: Node | null;
      while ((n = walker.nextNode())) {
        const t = n as Text;
        if (!t.textContent || !t.textContent.trim()) continue;
        const r = (t.parentElement as HTMLElement).getBoundingClientRect();
        if (r.bottom >= rootRect.top) {
          node = t;
          off = 0;
          break;
        }
      }
    }
    if (!node) return 0;
    // 累计该节点之前的字符数
    const walker2 = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
    let total = 0;
    let cur: Node | null;
    while ((cur = walker2.nextNode())) {
      if (cur === node) return total + Math.min(off, (node.textContent || "").length);
      total += (cur.textContent || "").length;
    }
    return total;
  }, []);

  // ── 保存进度（章节 + 字符偏移） ──
  const saveProgress = useCallback(() => {
    const pos = currentCharOffset();
    api.setNovelProgress(novel.id, chapterIdx, pos).catch(() => {});
  }, [novel.id, chapterIdx, currentCharOffset]);
  // 用 ref 保存最新 saveProgress，供卸载前调用
  const saveRef = useRef(saveProgress);
  saveRef.current = saveProgress;

  // 滚动防抖保存（1.2s 无滚动）
  useEffect(() => {
    const root = scrollRef.current;
    if (!root) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onScroll = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => saveRef.current(), 1200);
    };
    root.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      if (timer) clearTimeout(timer);
      root.removeEventListener("scroll", onScroll);
    };
  }, [chapterIdx]);

  // ── 恢复滚动位置（章节加载/首屏时） ──
  const scrollToOffset = useCallback(
    (target: number) => {
      const content = contentRef.current;
      const root = scrollRef.current;
      if (!content || !root) return;
      if (target <= 0) {
        root.scrollTop = 0;
        return;
      }
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      let total = 0;
      let cur: Node | null;
      while ((cur = walker.nextNode())) {
        const len = (cur.textContent || "").length;
        if (total + len >= target) {
          const inner = Math.min(target - total, len);
          try {
            const range = document.createRange();
            range.setStart(cur, inner);
            range.collapse(true);
            const rect = range.getBoundingClientRect();
            const rootRect = root.getBoundingClientRect();
            root.scrollTop = root.scrollTop + (rect.top - rootRect.top) - root.clientHeight * 0.25;
          } catch {
            root.scrollTop = 0;
          }
          return;
        }
        total += len;
      }
    },
    [],
  );

  // ── 章节加载 ──
  const loadChapter = useCallback(
    async (idx: number, restorePos: number) => {
      setChapterIdx(idx);
      setLoading(true);
      setError("");
      try {
        const text = await api.getNovelChapterContent(novel.id, idx);
        setContent(text);
        // 内容渲染后恢复滚动（双重 rAF 等待布局稳定）
        requestAnimationFrame(() => {
          requestAnimationFrame(() => scrollToOffset(restorePos));
        });
      } catch (e) {
        setError(String(e));
        setContent("");
      } finally {
        setLoading(false);
      }
    },
    [novel.id, scrollToOffset],
  );

  // 首次进入：加载起始章节并续读
  useEffect(() => {
    const pos =
      chapterIdx === initialIdx && novel.reading_pos > 0 ? novel.reading_pos : 0;
    loadChapter(initialIdx, pos);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 章节切换：先保存旧章节进度，再加载新章节
  const switchChapter = (idx: number) => {
    if (idx < 0 || idx >= novel.chapters.length || idx === chapterIdx) return;
    saveRef.current();
    setShowToc(false);
    loadChapter(idx, 0);
  };

  // ── 退出：保存进度 ──
  const handleClose = () => {
    saveRef.current();
    onClose();
  };

  // Q 键退出（输入框内不触发）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      if (e.key.toLowerCase() === "q") handleClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const changeFont = (delta: number) => {
    setFontSizeRaw((prev) => {
      const cur = Math.min(26, Math.max(13, parseInt(prev, 10) || 17));
      return String(Math.min(26, Math.max(13, cur + delta)));
    });
  };

  const chapter = novel.chapters[chapterIdx];

  return (
    <div className="reader-overlay novel-reader">
      {/* 顶部工具栏 */}
      <div className="reader-toolbar">
        <button className="btn-ghost" onClick={handleClose} title="返回书架（Q）">
          ← 返回
        </button>
        <div className="reader-title" title={novel.title}>
          {novel.title}
          <span className="reader-meta" style={{ marginLeft: 8 }}>
            {chapter ? chapter.title : "加载中…"}
          </span>
        </div>
        <button className="btn-ghost" onClick={() => changeFont(-1)} title="减小字号">
          A-
        </button>
        <button className="btn-ghost" onClick={() => changeFont(1)} title="增大字号">
          A+
        </button>
        <button className="btn-ghost" onClick={() => setShowToc((s) => !s)} title="章节列表">
          {showToc ? "收起目录" : "目录"}
        </button>
      </div>

      {/* 章节侧栏 */}
      {showToc && (
        <div className="novel-toc">
          <div className="novel-toc-title">目录（{novel.chapters.length} 章）</div>
          <div className="novel-toc-list">
            {novel.chapters.map((ch, idx) => (
              <div
                key={ch.id}
                className={`novel-toc-item${idx === chapterIdx ? " active" : ""}`}
                onClick={() => switchChapter(idx)}
              >
                {ch.title || `第 ${idx + 1} 章`}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 正文区 */}
      <div className="reader-scroll novel-scroll" ref={scrollRef}>
        <div className="novel-content" ref={contentRef} style={{ fontSize }}>
          {chapter && <h1 className="novel-chapter-title">{chapter.title}</h1>}
          {loading && <div className="novel-loading">加载中…</div>}
          {error && <div className="shelf-error">{error}</div>}
          {!loading && !error && content}
        </div>
        <div className="reader-end" onClick={handleClose}>
          全书完，点击返回书架
        </div>
      </div>

      {/* 底部章节导航 */}
      <div className="reader-controls">
        <button
          className="reader-nav-btn"
          disabled={chapterIdx <= 0}
          onClick={() => switchChapter(chapterIdx - 1)}
        >
          上一章
        </button>
        <span className="reader-pageno">
          {chapterIdx + 1} / {novel.chapters.length}
        </span>
        <button
          className="reader-nav-btn"
          disabled={chapterIdx >= novel.chapters.length - 1}
          onClick={() => switchChapter(chapterIdx + 1)}
        >
          下一章
        </button>
      </div>
    </div>
  );
}
