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

  // ── 移动端全屏策略（一次性考虑所有情况）：
  // 1) 进入播放器时尝试自动全屏（播放点击属于用户手势，effect 内大概率仍有效）→ 视频撑满屏幕
  // 2) 若自动全屏被浏览器拒绝（手势已失效）→ 视频上显示明显的"⛶ 全屏"按钮兜底，点一下即可
  // 3) 全屏后横屏视频在竖屏设备上由浏览器提示旋转，转过来即填满
  // 4) 桌面端不自动全屏，用原生控件/F 键
  const [fsActive, setFsActive] = useState(false);
  const [showFsBtn, setShowFsBtn] = useState(false);
  const [rotateHint, setRotateHint] = useState("");

  // 全屏状态跟踪（自动全屏/原生按钮/手动按钮共用）
  useEffect(() => {
    const onChange = () => {
      const active = !!document.fullscreenElement;
      setFsActive(active);
      if (active) setShowFsBtn(false);
    };
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  // 自动全屏（移动端、full 模式、视频就绪后尝试一次）
  useEffect(() => {
    if (!isMobile || isMini || !playUrl || fsActive) return;
    const el = wrapRef.current;
    if (!el) return;
    let cancelled = false;
    const tryFs = () => {
      if (cancelled || document.fullscreenElement) return;
      // 延迟一拍，等视频元素布局完成后再请求（布局尺寸为 0 时全屏会异常）
      requestAnimationFrame(() => {
        if (cancelled || document.fullscreenElement) return;
        const p = el.requestFullscreen?.().catch(() => {});
        // 部分浏览器拒绝时 requestFullscreen 返回 rejected promise；再兜一层延时判断
        if (p) {
          p.catch(() => {
            if (!cancelled && !document.fullscreenElement) setShowFsBtn(true);
          });
        }
        // 若 600ms 后仍未进入全屏（老浏览器可能不 reject 也不进入）→ 显示兜底按钮
        window.setTimeout(() => {
          if (!cancelled && !document.fullscreenElement) setShowFsBtn(true);
        }, 600);
      });
    };
    tryFs();
    return () => {
      cancelled = true;
    };
  }, [isMobile, isMini, playUrl, fsActive]);

  // 旋转提示：视频宽高比与设备方向不匹配时提示旋转（移动端、非全屏时也提示）
  useEffect(() => {
    if (!isMobile || isMini || !playUrl) return;
    const vw = video.frame_width || videoRef.current?.videoWidth || 0;
    const vh = video.frame_height || videoRef.current?.videoHeight || 0;
    if (!vw || !vh) return;
    const videoLandscape = vw > vh;
    const portrait = window.innerHeight > window.innerWidth;
    if (videoLandscape && portrait) {
      setRotateHint("↻ 旋转手机横屏观看，效果更佳");
    } else if (!videoLandscape && !portrait) {
      setRotateHint("↻ 旋转手机竖屏观看，效果更佳");
    } else {
      setRotateHint("");
    }
    const t = window.setTimeout(() => setRotateHint(""), 4000);
    return () => window.clearTimeout(t);
  }, [isMobile, isMini, playUrl, video.frame_width, video.frame_height]);

  // 字幕（ass/srt 由 jassub/libass WASM 渲染到覆盖层 canvas）
  const subResizeRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!video.subtitle_path || !playUrl) return;
    let cancelled = false;

    // 计算视频实际显示区域（object-fit:contain 后的内容框，排除黑边），
    // 让字幕 canvas 与视频内容严格对齐
    const layoutCanvas = (canvas: HTMLCanvasElement, videoEl: HTMLVideoElement) => {
      const container = wrapRef.current?.parentElement ?? videoEl.parentElement;
      const cw = container?.clientWidth ?? videoEl.clientWidth;
      const ch = container?.clientHeight ?? videoEl.clientHeight;
      const vw = videoEl.videoWidth;
      const vh = videoEl.videoHeight;
      let w = cw;
      let h = ch;
      if (vw && vh && cw && ch) {
        const scale = Math.min(cw / vw, ch / vh);
        w = Math.max(1, Math.round(vw * scale));
        h = Math.max(1, Math.round(vh * scale));
      }
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      canvas.style.left = "50%";
      canvas.style.top = "50%";
      canvas.style.transform = "translate(-50%, -50%)";
      canvas.width = w;
      canvas.height = h;
    };

    (async () => {
      try {
        const content = await api.getVideoSubtitle(video.id);
        if (cancelled) return;
        const videoEl = videoRef.current;
        const canvas = subCanvasRef.current;
        if (!videoEl || !canvas) return;
        layoutCanvas(canvas, videoEl);
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
        // 窗口/全屏尺寸变化时重排字幕层
        const onResize = () => {
          if (!cancelled && videoRef.current && subCanvasRef.current) {
            layoutCanvas(subCanvasRef.current, videoRef.current);
            inst.resize?.();
          }
        };
        subResizeRef.current = onResize;
        window.addEventListener("resize", onResize);
        document.addEventListener("fullscreenchange", onResize);
      } catch (e) {
        console.error("字幕加载失败:", e);
      }
    })();
    return () => {
      cancelled = true;
      if (subResizeRef.current) {
        window.removeEventListener("resize", subResizeRef.current);
        document.removeEventListener("fullscreenchange", subResizeRef.current);
        subResizeRef.current = null;
      }
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

  // ── 播完提示：有下一集 → 5s 倒计时自动连播（可取消）；单集 → 重播/关闭 ──
  const [endState, setEndState] = useState<"next" | "replay" | null>(null);
  const [countdown, setCountdown] = useState(0);
  const countdownRef = useRef<number | null>(null);

  const clearCountdown = useCallback(() => {
    if (countdownRef.current) {
      window.clearInterval(countdownRef.current);
      countdownRef.current = null;
    }
    setEndState(null);
  }, []);

  const replay = useCallback(() => {
    clearCountdown();
    const el = videoRef.current;
    if (el) {
      el.currentTime = 0;
      el.play().catch(() => {});
    }
  }, [clearCountdown]);

  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    const onEnded = () => {
      if (isMini) {
        // 迷你窗不做倒计时，直接连播
        goNext();
        return;
      }
      if (hasNext) {
        setEndState("next");
        setCountdown(5);
        if (countdownRef.current) window.clearInterval(countdownRef.current);
        countdownRef.current = window.setInterval(() => {
          setCountdown((c) => {
            if (c <= 1) {
              if (countdownRef.current) {
                window.clearInterval(countdownRef.current);
                countdownRef.current = null;
              }
              setEndState(null);
              goNext();
              return 0;
            }
            return c - 1;
          });
        }, 1000);
      } else {
        setEndState("replay");
      }
    };
    el.addEventListener("ended", onEnded);
    return () => {
      el.removeEventListener("ended", onEnded);
      if (countdownRef.current) {
        window.clearInterval(countdownRef.current);
        countdownRef.current = null;
      }
    };
  }, [goNext, hasNext, isMini]);

  // 切换视频/关闭时清除提示与倒计时
  useEffect(() => {
    setEndState(null);
    setCountdown(0);
    if (countdownRef.current) {
      window.clearInterval(countdownRef.current);
      countdownRef.current = null;
    }
  }, [playUrl, video.id]);

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
          else if (endState) clearCountdown();
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
    endState,
    clearCountdown,
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
            {/* 移动端兜底：自动全屏失败时显示明显"全屏"按钮 */}
            {!isMini && isMobile && !fsActive && showFsBtn && (
              <button
                className="pv-fs-btn"
                onClick={(e) => {
                  e.stopPropagation();
                  const el = wrapRef.current;
                  if (el) {
                    const p = el.requestFullscreen?.();
                    p?.catch?.(() => {});
                  }
                  setShowFsBtn(false);
                }}
              >
                ⛶ 全屏观看
              </button>
            )}
            {/* 旋转提示（视频方向与设备方向不匹配时短暂提示） */}
            {!isMini && isMobile && rotateHint && (
              <div className="pv-rotate-hint">{rotateHint}</div>
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

      {/* 播完提示：连播倒计时 / 重播 */}
      {!isMini && endState === "next" && hasNext && (
        <div key="end-next" className="pv-end-overlay">
          <div className="pv-end-box">
            <div className="pv-end-title">即将播放下一集</div>
            <div className="pv-end-sub">
              <span className="pv-end-count">{countdown}</span> 秒后自动连播 · {playlist[index + 1].title}
            </div>
            <div className="pv-end-actions">
              <button className="btn btn-sm btn-primary" onClick={() => { clearCountdown(); goNext(); }}>
                立即播放
              </button>
              <button className="btn btn-sm" onClick={clearCountdown}>取消</button>
            </div>
          </div>
        </div>
      )}
      {!isMini && endState === "replay" && (
        <div key="end-replay" className="pv-end-overlay">
          <div className="pv-end-box">
            <div className="pv-end-title">播放结束</div>
            <div className="pv-end-actions">
              <button className="btn btn-sm btn-primary" onClick={replay}>重播</button>
              <button className="btn btn-sm" onClick={onClose}>关闭</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
