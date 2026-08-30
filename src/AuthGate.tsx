// 浏览器/局域网访问口令门：桌面模式直接放行，浏览器模式需输入口令。
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { checkAuth, isWeb, login, UNAUTHORIZED_EVENT } from "./api";

export default function AuthGate({ children }: { children: ReactNode }) {
  const [checking, setChecking] = useState(isWeb);
  const [authed, setAuthed] = useState(!isWeb);
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    if (!isWeb) return;
    let cancelled = false;
    checkAuth()
      .then((s) => {
        if (!cancelled) {
          setAuthed(s.authed);
          setChecking(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setAuthed(false);
          setChecking(false);
        }
      });
    const onUnauthorized = () => setAuthed(false);
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => {
      cancelled = true;
      window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    };
  }, []);

  if (checking) {
    return (
      <div style={styles.gate}>
        <div style={styles.box}>
          <div style={styles.loading}>正在连接…</div>
        </div>
      </div>
    );
  }

  if (!authed) {
    return (
      <div style={styles.gate}>
        <div style={styles.box}>
          <h2 style={styles.title}>休闲时光</h2>
          <p style={styles.sub}>请输入访问口令以继续</p>
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              setError("");
              try {
                await login(password);
                setAuthed(true);
              } catch (err) {
                setError(String(err));
              }
            }}
          >
            <input
              type="password"
              value={password}
              autoFocus
              placeholder="6 位数字口令"
              onChange={(e) => setPassword(e.target.value)}
              style={styles.input}
            />
            <button type="submit" style={styles.button}>
              进入
            </button>
          </form>
          {error && <div style={styles.error}>{error}</div>}
        </div>
      </div>
    );
  }

  return <>{children}</>;
}

const styles: Record<string, React.CSSProperties> = {
  gate: {
    position: "fixed",
    inset: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    background: "#141414",
    color: "#eee",
    zIndex: 9990,
  },
  box: {
    width: "min(360px, 90vw)",
    background: "#1e1e1e",
    border: "1px solid #333",
    borderRadius: 10,
    padding: "32px 28px",
    textAlign: "center",
    boxShadow: "0 8px 32px rgba(0,0,0,.5)",
  },
  title: { margin: 0, fontSize: 22, fontWeight: 700 },
  sub: { color: "#999", fontSize: 13, margin: "8px 0 20px" },
  input: {
    width: "100%",
    boxSizing: "border-box",
    padding: "10px 12px",
    fontSize: 16,
    textAlign: "center",
    letterSpacing: 2,
    background: "#141414",
    color: "#eee",
    border: "1px solid #444",
    borderRadius: 6,
    outline: "none",
  },
  button: {
    width: "100%",
    marginTop: 12,
    padding: "10px 0",
    fontSize: 15,
    background: "#4a90d9",
    color: "#fff",
    border: "none",
    borderRadius: 6,
    cursor: "pointer",
  },
  error: { marginTop: 12, color: "#e07a7a", fontSize: 13 },
  loading: { color: "#999", fontSize: 14, padding: "20px 0" },
};
