// ============================================================
// 休闲时光 - 传输层（原 api.ts 拆分）
//
// 双传输层：桌面窗口走 Tauri IPC（invoke），浏览器/局域网走 HTTP。
//   - isWeb：检测是否运行于浏览器（无 Tauri 注入）
//   - call()：统一命令调用，浏览器模式 POST /api/{cmd}
//   - 口令：浏览器模式登录后 token 存 localStorage，请求带 X-Auth
//
// 后端命令层冲突时以模块语义前缀区分（见 commands.ts）：
//   - 漫画：get_comic_cover_data_url / open_comic_directory / batch_set_sort_order
//   - 视频：get_video_cover_data_url / open_video_folder / batch_set_video_sort_order
// ============================================================
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AuthStatus } from "./models";

// ══════════════════════════════════════════════════════════
//  传输层
// ══════════════════════════════════════════════════════════

/** 是否浏览器模式（无 Tauri 注入时判定） */
export const isWeb =
  typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window);

/** 浏览器模式访问口令存储 key（与 data.ts 注册的 "web-token" 对应） */
const TOKEN_KEY = "leisure-web-token";

/** 当前访问口令（浏览器模式；桌面模式为空） */
export function getToken(): string {
  try {
    return localStorage.getItem(TOKEN_KEY) || "";
  } catch {
    return "";
  }
}

/** 保存口令并（必要时）重建 SSE 连接 */
export function setToken(t: string) {
  try {
    localStorage.setItem(TOKEN_KEY, t);
  } catch {}
  resetSse();
}

export function clearToken() {
  try {
    localStorage.removeItem(TOKEN_KEY);
  } catch {}
}

/** 未授权事件（口令失效/被改时触发，App 显示登录门） */
export const UNAUTHORIZED_EVENT = "leisure-unauthorized";

/** 鉴权请求头（浏览器模式） */
export function authHeaders(): Record<string, string> {
  const t = getToken();
  return t ? { "x-auth": t } : {};
}

/** 浏览器模式下的 401 统一处理 */
export function handleUnauthorized() {
  clearToken();
  window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
}

/** snake_case → camelCase（桌面 invoke 需要 camelCase 参数名） */
function toCamel(key: string): string {
  return key.replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
}

/**
 * 统一命令调用：
 *  - 桌面：Tauri invoke（参数名自动转 camelCase，Tauri 会映射回 snake_case）
 *  - 浏览器：POST /api/{cmd}，JSON 参数（snake_case 原样传递），返回 { ok, data | error }
 */
export async function call<T>(
  cmd: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (!isWeb) {
    const camelArgs: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(args)) camelArgs[toCamel(k)] = v;
    return invoke<T>(cmd, camelArgs);
  }
  const res = await fetch(`/api/${cmd}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body: JSON.stringify(args),
  });
  if (res.status === 401) {
    handleUnauthorized();
  }
  let json: { ok?: boolean; data?: T; error?: string } | null = null;
  try {
    json = await res.json();
  } catch {}
  if (!json || json.ok !== true) {
    throw new Error(json?.error || `请求失败 (HTTP ${res.status})`);
  }
  return json.data as T;
}

// ══════════════════════════════════════════════════════════
//  认证（浏览器模式）
// ══════════════════════════════════════════════════════════

/** 查询口令状态与当前请求是否已认证 */
export async function checkAuth(): Promise<AuthStatus> {
  if (!isWeb) return { enabled: true, authed: true };
  const res = await fetch("/api/auth/status", { headers: authHeaders() });
  const json = await res.json().catch(() => null);
  return json ?? { enabled: false, authed: false };
}

/** 口令登录，成功后保存 token */
export async function login(password: string): Promise<void> {
  const res = await fetch("/api/auth/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
  });
  const json = await res.json().catch(() => null);
  if (!json || json.ok !== true) {
    throw new Error(json?.error || `登录失败 (HTTP ${res.status})`);
  }
  setToken(json.token as string);
}

// ══════════════════════════════════════════════════════════
//  事件（scan-done 等；onEvent 供 events.ts 使用）
// ══════════════════════════════════════════════════════════

let sse: EventSource | null = null;
const sseListeners = new Map<string, Set<(data: unknown) => void>>();

function resetSse() {
  if (!isWeb) return;
  if (sse) {
    sse.close();
    sse = null;
  }
}

/** 确保 SSE 已连接（浏览器模式；带口令 query，401 会自动重连） */
function ensureSse() {
  if (!isWeb || sse) return;
  sse = new EventSource(
    `/api/events?token=${encodeURIComponent(getToken() || "")}`,
  );
  sse.onerror = () => {
    // EventSource 内置自动重连；口令失效时仅影响事件推送，不阻塞页面
  };
}

/** 订阅命名事件（浏览器模式用 SSE，桌面模式用 tauri listen） */
export function onEvent<T>(
  name: string,
  cb: (data: T) => void,
): Promise<() => void> {
  if (!isWeb) {
    return listen<T>(name, (event) => cb(event.payload));
  }
  ensureSse();
  let set = sseListeners.get(name);
  if (!set) {
    set = new Set();
    sseListeners.set(name, set);
    sse!.addEventListener(name, (ev) => {
      let data: unknown = null;
      try {
        data = JSON.parse((ev as MessageEvent).data);
      } catch {}
      const list = sseListeners.get(name);
      if (list) list.forEach((f) => f(data));
    });
  }
  const fn = cb as (data: unknown) => void;
  set.add(fn);
  return Promise.resolve(() => {
    set.delete(fn);
  });
}

// ══════════════════════════════════════════════════════════
//  流 URL（浏览器模式）
// ══════════════════════════════════════════════════════════

/** 浏览器模式视频流 URL（HTTP Range 支持拖动进度条） */
export function videoStreamUrl(videoId: string): string {
  return `/api/video/stream/${encodeURIComponent(videoId)}?token=${encodeURIComponent(
    getToken(),
  )}`;
}

/** 浏览器模式图片二进制流 URL（替代 base64，浏览器原生解码；桌面模式返回空串走原 invoke 路径） */
export function imgUrl(segments: string[]): string {
  if (!isWeb) return "";
  const q = getToken() ? `?token=${encodeURIComponent(getToken())}` : "";
  return `/api/img/${segments.map(encodeURIComponent).join("/")}${q}`;
}

/** 浏览器模式故事会 PDF URL（pdf.js 按 Range 分块拉取，免全量 base64 传输） */
export function storyclubPdfUrl(issueId: string): string {
  return `/api/storyclub/pdf/${encodeURIComponent(issueId)}?token=${encodeURIComponent(
    getToken(),
  )}`;
}
