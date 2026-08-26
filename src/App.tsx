import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type DeadlockStatus = {
  deadlockRunning: boolean;
  deadlockPath: string | null;
  consoleLogPath: string | null;
  consoleLogExists: boolean;
  cfgDirExists: boolean;
  source: "legacy-config" | "steam-default" | "not-found";
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
      </section>

      {error && <div className="error-box">Backend error: {error}</div>}

      <footer>
        No background polling. This screen only refreshes on launch or when you press Refresh.
      </footer>
    </main>
  );
}

export default App;
