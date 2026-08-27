import {
  useCallback,
  useEffect,
  useState,
} from "react";

import {
  invoke,
} from "@tauri-apps/api/core";

import {
  listen,
} from "@tauri-apps/api/event";

import {
  open,
} from "@tauri-apps/plugin-dialog";

type DeadlockStatus = {
  deadlockRunning: boolean;
  deadlockPath: string | null;
  consoleLogPath: string | null;
  consoleLogExists: boolean;
  cfgDirExists: boolean;

  source:
    | "user-config"
    | "not-found";
};

type DeadlockSetupState = {
  configuredPath: string | null;
  detectedPath: string | null;
  needsSetup: boolean;
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

function StatusDot({
  ok,
}: {
  ok: boolean;
}) {
  return (
    <span
      className={`status-dot ${
        ok ? "ok" : "off"
      }`}
      aria-hidden="true"
    />
  );
}

function App() {
  const [
    setup,
    setSetup,
  ] =
    useState<DeadlockSetupState | null>(
      null,
    );

  const [
    setupLoading,
    setSetupLoading,
  ] = useState(true);

  const [
    setupWorking,
    setSetupWorking,
  ] = useState(false);

  const [
    status,
    setStatus,
  ] =
    useState<DeadlockStatus>(
      EMPTY_STATUS,
    );

  const [
    lastPosition,
    setLastPosition,
  ] =
    useState<PositionSnapshot | null>(
      null,
    );

  const [
    loading,
    setLoading,
  ] = useState(false);

  const [
    error,
    setError,
  ] =
    useState<string | null>(null);

  const refresh =
    useCallback(async () => {
      setLoading(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockStatus>(
            "get_deadlock_status",
          );

        setStatus(next);
        const position =
          await invoke<
            PositionSnapshot | null
          >(
            "get_last_position",
          );

        if (position) {
          setLastPosition(
            position,
          );
        }
      } catch (reason) {
        setError(String(reason));
      } finally {
        setLoading(false);
      }
    }, []);

  useEffect(() => {
  let disposed = false;

  let unlisten:
    | (() => void)
    | undefined;

  async function initializePositionBridge() {
    /*
     * 1. Installer le listener AVANT
     * de récupérer la dernière position.
     */
    const cleanup =
      await listen<PositionSnapshot>(
        "deadlock-position",
        (event) => {
          if (disposed) {
            return;
          }

          console.log(
            "[SPLIT UI] Position received:",
            event.payload,
          );

          setLastPosition(
            event.payload,
          );
        },
      );

    if (disposed) {
      cleanup();
      return;
    }

    unlisten = cleanup;

    /*
     * 2. Récupérer une éventuelle position
     * déjà parsée avant l'installation
     * du listener React.
     */
    try {
      const existing =
        await invoke<
          PositionSnapshot | null
        >(
          "get_last_position",
        );

      if (
        !disposed &&
        existing
      ) {
        setLastPosition(
          existing,
        );
      }
    } catch (reason) {
      console.error(
        "[SPLIT UI] Failed to retrieve last position:",
        reason,
      );
    }
  }

  void initializePositionBridge();

  return () => {
    disposed = true;
    unlisten?.();
  };
}, []);

  useEffect(() => {
    let disposed = false;

    let unlisten:
      | (() => void)
      | undefined;

    void listen<PositionSnapshot>(
      "deadlock-position",
      (event) => {
        console.log(
          "[SPLIT UI] Position received:",
          event.payload,
        );

        setLastPosition(
          event.payload,
        );
      },
    ).then((cleanup) => {
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

  const confirmPath =
    useCallback(
      async (
        path: string,
      ) => {
        setSetupWorking(true);
        setError(null);

        try {
          const nextStatus =
            await invoke<DeadlockStatus>(
              "confirm_deadlock_path",
              {
                path,
              },
            );

          setStatus(nextStatus);

          setSetup({
            configuredPath: path,
            detectedPath: null,
            needsSetup: false,
          });
        } catch (reason) {
          setError(String(reason));
        } finally {
          setSetupWorking(false);
        }
      },
      [],
    );

  const chooseFolder =
    useCallback(async () => {
      setError(null);

      const selected =
        await open({
          directory: true,
          multiple: false,
          title:
            "Choose the Deadlock installation folder",
        });

      if (
        selected === null ||
        Array.isArray(selected)
      ) {
        return;
      }

      await confirmPath(selected);
    }, [confirmPath]);

  const rescan =
    useCallback(async () => {
      setSetupWorking(true);
      setError(null);

      try {
        const detected =
          await invoke<string | null>(
            "scan_deadlock_path",
          );

        setSetup({
          configuredPath: null,
          detectedPath: detected,
          needsSetup: true,
        });
      } catch (reason) {
        setError(String(reason));
      } finally {
        setSetupWorking(false);
      }
    }, []);

  /*
   * Premier démarrage :
   * scan en cours.
   */
  if (
    setupLoading ||
    setup === null
  ) {
    return (
      <main className="shell setup-shell">
        <section className="setup-card">
          <p className="eyebrow">
            SPLIT 2
          </p>

          <h1>
            Detecting Deadlock
          </h1>

          <p className="setup-description">
            Scanning your Steam
            libraries…
          </p>

          <div className="scan-loader" />
        </section>
      </main>
    );
  }

  /*
   * Aucun dossier encore confirmé.
   */
  if (setup.needsSetup) {
    const detected =
      setup.detectedPath;

    return (
      <main className="shell setup-shell">
        <section className="setup-card">
          <p className="eyebrow">
            SPLIT 2 · FIRST SETUP
          </p>

          <h1>
            Deadlock installation
          </h1>

          {detected ? (
            <>
              <p className="setup-description">
                SPLIT detected a
                Deadlock installation.
              </p>

              <div className="detected-folder">
                <span>
                  DETECTED FOLDER
                </span>

                <code>
                  {detected}
                </code>
              </div>

              <h2 className="setup-question">
                Is this the correct
                Deadlock folder?
              </h2>

              <div className="setup-actions">
                <button
                  className="primary-button"
                  type="button"
                  disabled={
                    setupWorking
                  }
                  onClick={() =>
                    void confirmPath(
                      detected,
                    )
                  }
                >
                  Yes, continue
                </button>

                <button
                  className="secondary-button"
                  type="button"
                  disabled={
                    setupWorking
                  }
                  onClick={() =>
                    void chooseFolder()
                  }
                >
                  No, choose folder
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="setup-description">
                SPLIT couldn't find
                Deadlock automatically.
              </p>

              <p className="setup-description">
                Select the main
                <strong>
                  {" "}Deadlock{" "}
                </strong>
                installation folder.
              </p>

              <div className="setup-actions">
                <button
                  className="primary-button"
                  type="button"
                  disabled={
                    setupWorking
                  }
                  onClick={() =>
                    void chooseFolder()
                  }
                >
                  Choose Deadlock folder
                </button>

                <button
                  className="secondary-button"
                  type="button"
                  disabled={
                    setupWorking
                  }
                  onClick={() =>
                    void rescan()
                  }
                >
                  Scan again
                </button>
              </div>
            </>
          )}

          {setupWorking && (
            <p className="setup-working">
              Checking installation…
            </p>
          )}

          {error && (
            <div className="error-box">
              {error}
            </div>
          )}
        </section>
      </main>
    );
  }

  /*
   * Installation configurée :
   * écran diagnostic actuel.
   */
  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">
            SPLIT 2
          </p>

          <h1>
            Deadlock bridge
          </h1>

          <p className="subtitle">
            Rust core diagnostic
            checkpoint
          </p>
        </div>

        <button
          className="refresh-button"
          type="button"
          onClick={() =>
            void refresh()
          }
          disabled={loading}
        >
          {loading
            ? "Checking…"
            : "Refresh"}
        </button>
      </header>

      <section className="hero-card">
        <div className="hero-status">
          <StatusDot
            ok={
              status.deadlockRunning
            }
          />

          <div>
            <span className="label">
              DEADLOCK
            </span>

            <strong>
              {status.deadlockRunning
                ? "Running"
                : "Not running"}
            </strong>
          </div>
        </div>

        <span className="source-pill">
          {status.source}
        </span>
      </section>

      <section
        className="status-grid"
        aria-label="Deadlock diagnostics"
      >
        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={Boolean(
                status.deadlockPath,
              )}
            />

            <span>
              Game folder
            </span>
          </div>

          <code>
            {status.deadlockPath ??
              "Not configured"}
          </code>
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={
                status.cfgDirExists
              }
            />

            <span>
              CFG directory
            </span>
          </div>

          <strong>
            {status.cfgDirExists
              ? "Ready"
              : "Missing"}
          </strong>
        </article>

        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot
              ok={
                status.consoleLogExists
              }
            />

            <span>
              Console log
            </span>
          </div>

          <code>
            {status.consoleLogPath ??
              "No Deadlock path available"}
          </code>
        </article>

        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot
              ok={Boolean(
                lastPosition,
              )}
            />

            <span>
              Last parsed position
            </span>
          </div>

          {lastPosition ? (
            <code>
              XYZ {lastPosition.x}{" "}
              {lastPosition.y}{" "}
              {lastPosition.z} ·
              ANG{" "}
              {lastPosition.pitch}{" "}
              {lastPosition.yaw}{" "}
              {lastPosition.roll}
            </code>
          ) : (
            <strong>
              Waiting for a new
              getpos response…
            </strong>
          )}
        </article>
      </section>

      {error && (
        <div className="error-box">
          Backend error: {error}
        </div>
      )}

      <footer>
        No background polling.
        File changes are handled
        by the native Rust watcher.
      </footer>
    </main>
  );
}

export default App;