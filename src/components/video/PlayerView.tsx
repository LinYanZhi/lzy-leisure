import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, isWeb, videoStreamUrl, type Video } from "../../api";
import { useIsMobile } from "../../useIsMobile";

interface Props {
  video: Video;
  onClose: () => void;
  /** 连播列表（同剧集有序视频）；空数组/缺省表示单集播放 */
  playlist?: Video[];
  /** 当前视频在连播列表中的下标 */
  index?: number;
  /** 切到另一集（自动连播 / 上/下一集按钮） */
  onSwitch?: (video: Video) => void;
  /** 播放器形态：full=全屏覆盖层；mini=迷你悬浮窗（视频不中断） */
  mode?: "full" | "mini";
  /** mini 窗口点击 → 恢复全屏 */
  onRestore?: () => void;
  /** 显式关闭（停止播放并销毁） */
  onStop?: () => void;
}

/** 倍速档位（按钮循环 + [ / ] 逐档微调共用） */
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];

/** 快捷键帮助列表（连播行仅在剧集播放时展示） */
const HELP_ROWS: [string, string][] = [
  ["空格 / K", "播放 / 暂停"],
  ["← / →（J / L）", "后退 / 快进 5 秒"],
  ["↑ / ↓", "音量 + / -"],
  ["M", "静音切换"],
  ["[ / ]", "降低 / 提高倍速"],
  ["F", "全屏 / 退出全屏"],
  ["P", "画中画"],
  ["N / Shift+N", "下一集 / 上一集"],
  ["?", "快捷键帮助"],
  ["Esc", "关闭播放器"],
];

/**
 * 沉浸式播放器：视频 + 字幕（jassub 覆盖层）+ 续播记忆。
 * 播放控制：倍速 / 快捷键 / 全屏 / 画中画 / 剧集连播（播完自动下一集）。
 */
export default function PlayerView({ video, onClose, playlist = [], index = 0, onSwitch, mode = "full", onRestore, onStop }: Props) {
  const [playUrl, setPlayUrl] = useState<string>();
  const [error, setError] = useState("");
  const [subEnabled, setSubEnabled] = useState(true);
  const [speed, setSpeed] = useState(1);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  // 移动端：控制栏显隐（3s 自动隐藏）+ 双击 seek 提示
  const isMobile = useIsMobile();
  const isMini = mode === "mini";
  const [controlsVisible, setControlsVisible] = useState(true);
  const [seekHint, setSeekHint] = useState<string>("");
  const hideTimerRef = useRef<number | null>(null);
  const lastTapRef = useRef<{ t: number; x: number } | null>(null);

  const videoRef = useRef<HTMLVideoElement | null>(null);
  const subCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const subInstRef = useRef<any>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  const hasPrev = index > 0 && playlist.length > 0 && !!onSwitch;
  const hasNext = index < playlist.length - 1 && !!onSwitch;

  // ── 移动端控制栏：播放中 3s 无操作自动隐藏；点击唤出 ──
  const showControls = useCallback((autoHide = true) => {
    setControlsVisible(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    if (autoHide && isMobile) {
      hideTimerRef.current = window.setTimeout(() => setControlsVisible(false), 3000);
    }
  }, [isMobile]);
  useEffect(() => () => { if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current); }, []);

  // seek 提示（双击快进/快退时显示 1s）
  const flashSeek = useCallback((hint: string) => {
    setSeekHint(hint);
    window.setTimeout(() => setSeekHint(""), 900);
  }, []);

  // ── 移动端手势（onWrapTap 定义在 seekBy/togglePlay 之后） ──

  // ── 屏幕常亮（播放期间） ──
  useEffect(() => {
    let lock: any = null;
    const acquire = () => {
      // @ts-ignore wakeLock 为较新 API
      if (isWeb && navigator.wakeLock?.request) {
        // @ts-ignore
        navigator.wakeLock.request("screen").then((l) => { lock = l; }).catch(() => {});
      }
    };
    acquire();
    return () => { lock?.release?.().catch(() => {}); };
  }, [isWeb, playUrl]);

  // ── 移动端进入即全屏：作用于视频容器（视频填满屏幕，横屏提示交给浏览器） ──
  useEffect(() => {
    if (!isMobile || isMini || !playUrl) return;
    const el = wrapRef.current;
    if (el && !document.fullscreenElement) {
      el.requestFullscreen?.().catch(() => {});
    }
  }, [isMobile, isMini, playUrl]);

  // 字幕（ass/srt 由 jassub/libass WASM 渲染到覆盖层 canvas）
  useEffect(() => {
    if (!video.subtitle_path || !playUrl) return;
    let cancelled = false;
    (async () => {
      try {
        const content = await api.getVideoSubtitle(video.id);
        if (cancelled) return;
        const videoEl = videoRef.current;
        const canvas = subCanvasRef.current;
        if (!videoEl || !canvas) return;
        const { default: JASSUB } = await import("jassub");
        const inst = new JASSUB({
          video: videoEl,
          canvas,
          subContent: content,
          queryFonts: "local",
        });
        await inst.ready.catch(() => {});
        if (cancelled) {
          inst.destroy().catch(() => {});
          return;
        }
        subInstRef.current = inst;
      } catch (e) {
        console.error("字幕加载失败:", e);
      }
    })();
    return () => {
      cancelled = true;
      subInstRef.current?.destroy?.().catch(() => {});
      subInstRef.current = null;
    };
  }, [video.id, video.subtitle_path, playUrl]);

  // 倍速：应用到 video 元素（切集后自动恢复当前档位）
  useEffect(() => {
    const el = videoRef.current;
    if (el) el.playbackRate = speed;
  }, [speed, playUrl]);

  const cycleSpeed = useCallback(() => {
    setSpeed((s) => SPEEDS[(SPEEDS.indexOf(s) + 1) % SPEEDS.length]);
  }, []);

  const speedStep = useCallback((dir: 1 | -1) => {
    setSpeed((s) => {
      const i = SPEEDS.indexOf(s);
      const j = Math.min(SPEEDS.length - 1, Math.max(0, i + dir));
      return SPEEDS[j];
    });
  }, []);

  // ── 播放控制辅助（读 ref，稳定引用） ──
  const togglePlay = useCallback(() => {
    const el = videoRef.current;
    if (!el) return;
    if (el.paused) el.play().catch(() => {});
    else el.pause();
  }, []);

  const seekBy = useCallback((delta: number) => {
    const el = videoRef.current;
    if (!el || !isFinite(el.duration)) return;
    el.currentTime = Math.min(Math.max(0, el.currentTime + delta), el.duration);
  }, []);

  // ── 移动端手势：单击=唤出/隐藏控制栏；双击左/右 1/3 = ±10s；中央双击=播放暂停 ──
  const onWrapTap = useCallback((e: React.MouseEvent | React.TouchEvent) => {
    if (!isMobile || isMini) return;
    const rect = wrapRef.current?.getBoundingClientRect();
    if (!rect) return;
    const now = Date.now();
    const x = "touches" in e ? e.touches[0].clientX : (e as React.MouseEvent).clientX;
    const last = lastTapRef.current;
    lastTapRef.current = { t: now, x };
    // 双击判定：300ms 内第二次点击
    if (last && now - last.t < 300 && Math.abs(x - last.x) < 60) {
      const third = rect.width / 3;
      if (x < rect.left + third) {
        seekBy(-10);
        flashSeek("⏪ 10 秒");
      } else if (x > rect.left + third * 2) {
        seekBy(10);
        flashSeek("⏩ 10 秒");
      } else {
        togglePlay();
      }
      lastTapRef.current = null;
      return;
    }
    // 单击：切换控制栏（等 300ms 看是否双击）
    window.setTimeout(() => {
      if (lastTapRef.current && lastTapRef.current.t === now) {
        lastTapRef.current = null;
        if (controlsVisible) setControlsVisible(false);
        else showControls();
      }
    }, 300);
  }, [isMobile, controlsVisible, showControls, seekBy, togglePlay, flashSeek]);

  const adjustVolume = useCallback((delta: number) => {
    const el = videoRef.current;
    if (!el) return;
    el.volume = Math.min(1, Math.max(0, el.volume + delta));
  }, []);

  const toggleMute = useCallback(() => {
    const el = videoRef.current;
    if (el) el.muted = !el.muted;
  }, []);

  // 全屏目标为视频容器（含字幕 canvas 与原生控制条一起全屏）
  const toggleFullscreen = useCallback(() => {
    const el = wrapRef.current;
    if (!el) return;
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      el.requestFullscreen().catch(() => {});
    }
  }, []);

  const pipSupported =
    typeof document !== "undefined" &&
    "pictureInPictureEnabled" in document &&
    document.pictureInPictureEnabled;

  const togglePip = useCallback(async () => {
    const el = videoRef.current;
    if (!el) return;
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
      } else {
        await el.requestPictureInPicture();
      }
    } catch (e) {
      console.error("画中画失败:", e);
    }
  }, []);

  // ── 剧集连播：上/下一集 ──
  const goPrev = useCallback(() => {
    if (hasPrev && onSwitch) onSwitch(playlist[index - 1]);
  }, [hasPrev, index, onSwitch, playlist]);

  const goNext = useCallback(() => {
    if (hasNext && onSwitch) onSwitch(playlist[index + 1]);
  }, [hasNext, index, onSwitch, playlist]);

  // 播完自动下一集
  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    const onEnded = () => goNext();
    el.addEventListener("ended", onEnded);
    return () => el.removeEventListener("ended", onEnded);
  }, [goNext]);

  // 续播：元数据加载后跳转到上次进度（跳过开头/结尾的边界值）
  useEffect(() => {
    const el = videoRef.current;
    if (!el || !playUrl) return;
    const onMeta = () => {
      const p = video.progress;
      if (p > 5 && el.duration > 0 && p < el.duration - 5) {
        el.currentTime = p;
      }
      // 切集后 src 变化不保证 autoplay 生效，显式拉起播放
      el.play().catch(() => {});
    };
    el.addEventListener("loadedmetadata", onMeta);
    return () => el.removeEventListener("loadedmetadata", onMeta);
  }, [playUrl, video.progress]);

  // 进度记忆：每 5 秒 + 暂停/播完/卸载时保存到后端
  useEffect(() => {
    const el = videoRef.current;
    if (!el || !playUrl) return;
    const save = () => {
      if (el.duration > 0 && el.currentTime > 0) {
        api.updateVideoProgress(video.id, el.currentTime).catch(() => {});
      }
    };
    const iv = setInterval(save, 5000);
    el.addEventListener("pause", save);
    el.addEventListener("ended", save);
    return () => {
      clearInterval(iv);
      el.removeEventListener("pause", save);
      el.removeEventListener("ended", save);
      save();
    };
  }, [playUrl, video.id]);

  // 播放地址：浏览器用 HTTP 流（Range），桌面用 asset 协议。
  // 切换视频时先清空地址，避免短暂继续播放上一集。
  useEffect(() => {
    let cancelled = false;
    setError("");
    setPlayUrl(undefined);
    if (isWeb) {
      setPlayUrl(videoStreamUrl(video.id));
    } else {
      api
        .getVideoPlayPath(video.id)
        .then((path) => {
          if (!cancelled) setPlayUrl(convertFileSrc(path));
        })
        .catch((e) => setError(String(e)));
    }
    return () => {
      cancelled = true;
    };
  }, [video.id]);

  // 全屏状态跟踪（按钮高亮）
  useEffect(() => {
    const onChange = () => setIsFullscreen(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  // 快捷键（空格/K 播放、方向键/JL 快进快退、上下音量、M 静音、
  // [ / ] 倍速、F 全屏、P 画中画、N/Shift+N 切集、? 帮助、Esc 关闭）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isMini) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const k = e.key.toLowerCase();
      switch (k) {
        case "escape":
          if (showHelp) setShowHelp(false);
          else onClose();
          break;
        case " ":
        case "spacebar":
        case "k":
          e.preventDefault();
          togglePlay();
          break;
        case "arrowleft":
        case "j":
          e.preventDefault();
          seekBy(-5);
          break;
        case "arrowright":
        case "l":
          e.preventDefault();
          seekBy(5);
          break;
        case "arrowup":
          e.preventDefault();
          adjustVolume(0.1);
          break;
        case "arrowdown":
          e.preventDefault();
          adjustVolume(-0.1);
          break;
        case "m":
          e.preventDefault();
          toggleMute();
          break;
        case "f":
          e.preventDefault();
          toggleFullscreen();
          break;
        case "p":
          e.preventDefault();
          void togglePip();
          break;
        case "[":
          e.preventDefault();
          speedStep(-1);
          break;
        case "]":
          e.preventDefault();
          speedStep(1);
          break;
        case "n":
          e.preventDefault();
          if (e.shiftKey) goPrev();
          else goNext();
          break;
        case "?":
          e.preventDefault();
          setShowHelp((h) => !h);
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    onClose,
    showHelp,
    isMini,
    togglePlay,
    seekBy,
    adjustVolume,
    toggleMute,
    toggleFullscreen,
    togglePip,
    speedStep,
    goPrev,
    goNext,
  ]);

  const res = video.frame_width && video.frame_height ? `${video.frame_width}×${video.frame_height}` : "";
  const speedLabel = String(speed).replace(/\.0+$/, "");
  const helpRows = playlist.length > 0 ? HELP_ROWS : HELP_ROWS.filter(([k]) => k !== "N / Shift+N");

  // 手机浏览器常见不支持/兼容差的格式（电脑端无影响）
  const MOBILE_WARN_FORMATS = new Set([
    "mkv", "avi", "wmv", "flv", "rmvb", "ts", "mpg", "mpeg", "vob", "f4v",
  ]);
  const videoFormat = (video.file_type || "").toLowerCase().replace(/^\./, "");
  const mobileWarn = MOBILE_WARN_FORMATS.has(videoFormat);

  return (
    <div
      className={isMini ? "pv-mini" : "pv-overlay"}
      onClick={isMini ? () => onRestore?.() : undefined}
    >
      {/* 全屏顶栏（仅 full 模式；mini 模式不渲染，但视频元素跨模式存活） */}
      {!isMini && (
        <div key="top" className={`pv-topbar${isMobile && !controlsVisible ? " pv-topbar-hidden" : ""}`}>
          <button className="btn btn-sm" onClick={onClose}>← 返回</button>
          <div className="pv-title" title={video.title}>{video.title}</div>
          {playlist.length > 0 && (
            <span className="pv-ep-pos" title="剧集连播位置">{index + 1} / {playlist.length}</span>
          )}
          <div className="pv-meta">
            {[video.duration, res, video.file_size].filter(Boolean).join(" · ")}
          </div>
          {hasPrev && (
            <button className="btn btn-sm" onClick={goPrev} title="上一集（Shift+N）">← 上集</button>
          )}
          {hasNext && (
            <button className="btn btn-sm" onClick={goNext} title="下一集（N；播完自动连播）">下集 →</button>
          )}
          {video.subtitle_path && (
            <button
              className={`btn btn-sm pv-sub-toggle${subEnabled ? " on" : ""}`}
              onClick={() => setSubEnabled((v) => !v)}
              title="切换字幕显示"
            >
              字幕 {subEnabled ? "开" : "关"}
            </button>
          )}
          <button className="btn btn-sm" onClick={cycleSpeed} title="倍速（[ / ]）">
            {speedLabel}x
          </button>
          {pipSupported && (
            <button className="btn btn-sm" onClick={() => void togglePip()} title="画中画（P）">
              画中画
            </button>
          )}
          <button
            className={`btn btn-sm pv-fs${isFullscreen ? " on" : ""}`}
            onClick={toggleFullscreen}
            title="全屏（F）"
          >
            全屏
          </button>
          <button className="btn btn-sm pv-help-btn" onClick={() => setShowHelp(true)} title="快捷键（?）">
            ?
          </button>
        </div>
      )}

      {/* 视频区（full/mini 共用一个元素实例，播放不中断） */}
      <div key="video" className="pv-video-wrap">
        {!isMini && mobileWarn && (
          <div className="pv-mobile-warn">
            当前为 {video.file_type || videoFormat} 格式，手机浏览器可能无法播放，建议在电脑上播放
          </div>
        )}
        {playUrl ? (
          <div
            className="pv-video-holder"
            ref={wrapRef}
            onTouchEnd={isMini ? undefined : onWrapTap}
            onClick={isMini ? undefined : onWrapTap}
          >
            <video
              ref={videoRef}
              className="pv-video"
              src={playUrl}
              controls={!isMini}
              autoPlay
              playsInline
            />
            {video.subtitle_path && (
              <canvas
                ref={subCanvasRef}
                className={`pv-sub-canvas${subEnabled ? "" : " hidden"}`}
              />
            )}
            {!isMini && isMobile && seekHint && (
              <div className="pv-seek-hint">{seekHint}</div>
            )}
          </div>
        ) : (
          <div className="pv-loading">正在加载视频…</div>
        )}
      </div>

      {!isMini && error && (
        <div key="bottom" className="pv-error" style={{ padding: "0 16px 12px" }}>{error}</div>
      )}

      {!isMini && showHelp && (
        <div
          key="help"
          className="pv-help-overlay"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowHelp(false);
          }}
        >
          <div className="pv-help-box">
            <div className="pv-help-title">
              播放器快捷键
              <button className="btn btn-sm" onClick={() => setShowHelp(false)}>关闭</button>
            </div>
            <div className="pv-help-grid">
              {helpRows.map(([k, d]) => (
                <div key={k} className="pv-help-row">
                  <kbd className="pv-kbd">{k}</kbd>
                  <span>{d}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* mini 底部条：标题 + 关闭（停止播放） */}
      {isMini && (
        <div key="mini" className="pv-mini-bar" onClick={(e) => e.stopPropagation()}>
          <span className="pv-mini-title" title={video.title}>{video.title}</span>
          <button className="pv-mini-close" onClick={onStop} title="停止播放">×</button>
        </div>
      )}
    </div>
  );
}
