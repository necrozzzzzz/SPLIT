import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type DeadlockStatus = {
  deadlockRunning: boolean;
  deadlockPath: string | null;
  consoleLogPath: string | null;
  consoleLogExists: boolean;
  cfgDirExists: boolean;
  source: "legacy-config" | "steam-default" | "not-found";
};

type PositionSnapshot = {
  x: number;
  y: number;
  z: number;
  pitch: number;
  yaw: number;
  roll: number;
};

const EMPTY_STATUS: DeadlockStatus = {
  deadlockRunning: false,
  deadlockPath: null,
  consoleLogPath: null,
  consoleLogExists: false,
  cfgDirExists: false,
  source: "not-found",
};

function StatusDot({ ok }: { ok: boolean }) {
  return <span className={`status-dot ${ok ? "ok" : "off"}`} aria-hidden="true" />;
}

function App() {
  const [status, setStatus] = useState<DeadlockStatus>(EMPTY_STATUS);
  const [lastPosition, setLastPosition] = useState<PositionSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      const next = await invoke<DeadlockStatus>("get_deadlock_status");
      setStatus(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<PositionSnapshot>("deadlock://position", (event) => {
      setLastPosition(event.payload);
    }).then((cleanup) => {
      if (disposed) {
        cleanup();
      } else {
        unlisten = cleanup;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">SPLIT 2</p>
          <h1>Deadlock bridge</h1>
          <p className="subtitle">Rust core diagnostic checkpoint</p>
        </div>

        <button className="refresh-button" type="button" onClick={() => void refresh()} disabled={loading}>
          {loading ? "Checking…" : "Refresh"}
        </button>
      </header>

      <section className="hero-card">
        <div className="hero-status">
          <StatusDot ok={status.deadlockRunning} />
          <div>
            <span className="label">DEADLOCK</span>
            <strong>{status.deadlockRunning ? "Running" : "Not running"}</strong>
          </div>
        </div>
        <span className="source-pill">{status.source}</span>
      </section>

      <section className="status-grid" aria-label="Deadlock diagnostics">
        <article className="status-card">
          <div className="status-heading">
            <StatusDot ok={Boolean(status.deadlockPath)} />
            <span>Game folder</span>
          </div>
          <code>{status.deadlockPath ?? "Not detected"}</code>
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot ok={status.cfgDirExists} />
            <span>CFG directory</span>
          </div>
          <strong>{status.cfgDirExists ? "Ready" : "Missing"}</strong>
        </article>

        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot ok={status.consoleLogExists} />
            <span>Console log</span>
          </div>
          <code>{status.consoleLogPath ?? "No Deadlock path available"}</code>
        </article>

        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot ok={Boolean(lastPosition)} />
            <span>Last parsed position</span>
          </div>
          {lastPosition ? (
            <code>
              XYZ {lastPosition.x} {lastPosition.y} {lastPosition.z} · ANG {lastPosition.pitch} {lastPosition.yaw} {lastPosition.roll}
            </code>
          ) : (
            <strong>Waiting for a new getpos response…</strong>
          )}
        </article>
      </section>

      {error && <div className="error-box">Backend error: {error}</div>}

      <footer>
        No background polling. File changes are handled by the native Rust watcher and emitted to the UI only when needed.
      </footer>
    </main>
  );
}

export default App;
