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

type SaveFailedPayload = {
  slot: number;
  reason: string;
};

type HistoryState = {
  canUndo: boolean;
  canRedo: boolean;
};

type HistoryOperationResult = {
  preset: number;
  slots: Array<PositionSnapshot | null>;
  historyState: HistoryState;
  favoriteActive: boolean;
  performed: boolean;
};

type ActiveBankResult = {
  preset: number;
  slots: Array<PositionSnapshot | null>;
  favoriteActive: boolean;
};

type NotificationPosition =
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight";

type NotificationSettings = {
  enabled: boolean;
  position: NotificationPosition;
  durationMs: number;
};

const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  enabled: true,
  position: "topRight",
  durationMs: 1500,
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
    slots,
    setSlots,
  ] = useState<
    Array<PositionSnapshot | null>
  >(
    () =>
      Array.from(
        { length: 8 },
        () => null,
      ),
  );

  const [
    activePreset,
    setActivePreset,
  ] = useState(
    1,
  );

  const [
    historyState,
    setHistoryState,
  ] = useState<HistoryState>({
    canUndo: false,
    canRedo: false,
  });

  const [
    favoriteMode,
    setFavoriteMode,
  ] = useState(false);

  const [
    notificationSettings,
    setNotificationSettings,
  ] = useState<NotificationSettings>(
    DEFAULT_NOTIFICATION_SETTINGS,
  );

  const [
    notificationSettingsSaving,
    setNotificationSettingsSaving,
  ] = useState(false);

  const [
    savingSlot,
    setSavingSlot,
  ] = useState<number | null>(
    null,
  );

  const [
    loadingSlot,
    setLoadingSlot,
  ] = useState<number | null>(
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

    async function initializeSetup() {
      setSetupLoading(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockSetupState>(
            "get_deadlock_setup",
          );

        if (disposed) {
          return;
        }

        setSetup(next);

        if (!next.needsSetup) {
          await refresh();
        }
      } catch (reason) {
        if (!disposed) {
          setError(String(reason));
        }
      } finally {
        if (!disposed) {
          setSetupLoading(false);
        }
      }
    }

    void initializeSetup();

    return () => {
      disposed = true;
    };
  }, [refresh]);
    
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

    async function loadSlots() {
      try {
        const [
          saved,
          preset,
          history,
          favoriteActive,
        ] =
          await Promise.all([
            invoke<
              Array<PositionSnapshot | null>
            >(
              "get_slots",
            ),

            invoke<number>(
              "get_active_preset",
            ),

            invoke<HistoryState>(
              "get_history_state",
            ),

            invoke<boolean>(
              "get_favorite_mode",
            ),
          ]);


        if (!disposed) {
          setSlots(
            saved,
          );

          setActivePreset(
            preset,
          );

          setHistoryState(
            history,
          );

          setFavoriteMode(
            favoriteActive,
          );
        }
      } catch (reason) {
        if (!disposed) {
          setError(
            String(reason),
          );
        }
      }
    }

    void loadSlots();

    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<SaveFailedPayload>(
      "deadlock-save-failed",
      (event) => {
        if (!disposed) {
          setSavingSlot(null);
          setError(event.payload.reason);
        }
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

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<boolean>(
      "deadlock-favorite-mode",
      (event) => {
        if (!disposed) {
          setFavoriteMode(event.payload);
        }
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

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<HistoryState>(
      "deadlock-history-state",
      (event) => {
        if (!disposed) {
          setHistoryState(event.payload);
        }
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

    useEffect(() => {
    let disposed = false;

    let unlisten:
      | (() => void)
      | undefined;

    void listen<
      Array<PositionSnapshot | null>
    >(
      "deadlock-slots",
      (event) => {
        if (!disposed) {
          setSlots(
            event.payload,
          );

          setSavingSlot(
            null,
          );
        }
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

  useEffect(() => {
    let disposed = false;

    let unlisten:
      | (() => void)
      | undefined;

    void listen<number>(
      "deadlock-preset",
      (event) => {
        if (!disposed) {
          setActivePreset(
            event.payload,
          );
        }
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

  const saveCurrentToSlot =
  useCallback(
    async (
      slot: number,
    ) => {
      setSavingSlot(
        slot,
      );

      setError(
        null,
      );

      try {
        await invoke(
          "capture_slot",
          {
            slot,
          },
        );
      } catch (reason) {
        setSavingSlot(
          null,
        );

        setError(
          String(reason),
        );
      }
    },
    [],
  );

    const loadSavedSlot =
      useCallback(
        async (
          slot: number,
        ) => {
          setLoadingSlot(
            slot,
          );

          setError(
            null,
          );

          try {
            await invoke(
              "load_slot",
              {
                slot,
              },
            );
          } catch (reason) {
            setError(
              String(reason),
            );
          } finally {
            setLoadingSlot(
              null,
            );
          }
        },
        [],
      );

      const switchPreset =
  useCallback(
    async (
      preset: number,
    ) => {
      if (
        preset === activePreset &&
        !favoriteMode
      ) {
        return;
      }


      setError(
        null,
      );


      try {
        const saved =
          await invoke<
            Array<PositionSnapshot | null>
          >(
            "set_active_preset",
            {
              preset,
            },
          );


        setActivePreset(
          preset,
        );

        setSlots(
          saved,
        );

        setFavoriteMode(false);
      } catch (reason) {
        setError(
          String(reason),
        );
      }
    },
    [
      activePreset,
      favoriteMode,
    ],
  );

  const runHistoryAction =
    useCallback(
      async (
        command: "undo_last_action" | "redo_last_action",
      ) => {
        setError(null);

        try {
          const result =
            await invoke<HistoryOperationResult>(
              command,
            );
          setActivePreset(result.preset);
          setSlots(result.slots);
          setHistoryState(result.historyState);
          setFavoriteMode(result.favoriteActive);
        } catch (reason) {
          setError(String(reason));
        }
      },
      [],
    );

  const toggleFavorites =
    useCallback(async () => {
      setError(null);

      try {
        const result =
          await invoke<ActiveBankResult>(
            "toggle_favorite_mode",
          );
        setActivePreset(result.preset);
        setSlots(result.slots);
        setFavoriteMode(result.favoriteActive);
      } catch (reason) {
        setError(String(reason));
      }
    }, []);

  useEffect(() => {
    let disposed = false;

    async function loadNotificationSettings() {
      try {
        const saved =
          await invoke<NotificationSettings>(
            "get_notification_settings",
          );

        if (!disposed) {
          setNotificationSettings(saved);
        }
      } catch (reason) {
        if (!disposed) {
          setError(String(reason));
        }
      }
    }

    void loadNotificationSettings();

    return () => {
      disposed = true;
    };
    }, []);

  const updateNotificationSettings =
    useCallback(
      async (
        next: NotificationSettings,
      ) => {
        const previous = notificationSettings;
        setNotificationSettings(next);
        setNotificationSettingsSaving(true);
        setError(null);

        try {
          const saved =
            await invoke<NotificationSettings>(
              "update_notification_settings",
              { settings: next },
            );
          setNotificationSettings(saved);
        } catch (reason) {
          setNotificationSettings(previous);
          setError(String(reason));
        } finally {
          setNotificationSettingsSaving(false);
        }
      },
      [notificationSettings],
    );

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


    <section className="savestates-section">
      <div className="savestates-header">
        <div>
          <p className="label">
            SAVESTATES
          </p>

          <h2>
            Position slots
          </h2>
        </div>

        <span className="savestates-hint">
          Save Alt+F1–F8 · Load F1–F8
        </span>
      </div>


      <div className="preset-switcher">
        {[1, 2, 3, 4].map(
          (preset) => (
            <button
              key={preset}
              type="button"
              className={`preset-button ${
                activePreset === preset
                  && !favoriteMode
                  ? "active"
                  : ""
              }`}
              disabled={
                savingSlot !== null ||
                loadingSlot !== null
              }
              onClick={() =>
                void switchPreset(
                  preset,
                )
              }
            >
              Preset {preset}
            </button>
          ),
        )}
      </div>

      <button
        className={`favorite-mode-button ${
          favoriteMode ? "active" : ""
        }`}
        type="button"
        disabled={
          savingSlot !== null ||
          loadingSlot !== null
        }
        onClick={() =>
          void toggleFavorites()
        }
      >
        Favorites · F11
      </button>

      <div className="history-actions">
        <button
          className="preset-button"
          type="button"
          disabled={
            !historyState.canUndo ||
            savingSlot !== null ||
            loadingSlot !== null
          }
          onClick={() =>
            void runHistoryAction(
              "undo_last_action",
            )
          }
        >
          Undo&nbsp;&nbsp;F9
        </button>

        <button
          className="preset-button"
          type="button"
          disabled={
            !historyState.canRedo ||
            savingSlot !== null ||
            loadingSlot !== null
          }
          onClick={() =>
            void runHistoryAction(
              "redo_last_action",
            )
          }
        >
          Redo&nbsp;&nbsp;F10
        </button>
      </div>

      <div className="slots-grid">
        {slots.map(
          (
            position,
            index,
          ) => {
            const slot =
              index + 1;

            return (
              <article
                className="slot-card"
                key={slot}
              >
                <div className="slot-top">
                  <span className="slot-number">
                    {favoriteMode
                      ? "FAVORITE"
                      : "SLOT"}{" "}
                    {slot}
                  </span>

                  <span
                    className={`slot-state ${
                      position
                        ? "filled"
                        : ""
                    }`}
                  >
                    {position
                      ? "Saved"
                      : "Empty"}
                  </span>
                </div>

                            {position ? (
              <div className="slot-position">
                <code>
                  XYZ{" "}
                  {position.x.toFixed(2)}{" "}
                  {position.y.toFixed(2)}{" "}
                  {position.z.toFixed(2)}
                </code>

                <code>
                  ANG{" "}
                  {position.pitch.toFixed(2)}{" "}
                  {position.yaw.toFixed(2)}{" "}
                  {position.roll.toFixed(2)}
                </code>
              </div>
            ) : (
              <p className="slot-empty">
                No position saved
              </p>
            )}

            <div className="slot-shortcuts">
              <span>
                Load F{slot}
              </span>

              <span>
                Save Alt+F{slot}
              </span>
            </div>

            <div className="slot-actions">
              <button
                className="slot-save-button slot-load-button"
                type="button"
                disabled={
                  !position ||
                  loadingSlot !== null ||
                  savingSlot !== null
                }
                onClick={() =>
                  void loadSavedSlot(
                    slot,
                  )
                }
              >
                {loadingSlot === slot
                  ? "Loading…"
                  : "Load"}
              </button>

              <button
                className="slot-save-button"
                type="button"
                disabled={
                  savingSlot !== null ||
                  loadingSlot !== null
                }
                onClick={() =>
                  void saveCurrentToSlot(
                    slot,
                  )
                }
              >
                {savingSlot === slot
                  ? "Saving…"
                  : position
                    ? "Overwrite"
                    : "Save"}
              </button>
            </div>
              </article>
            );
          },
        )}
      </div>
    </section>

      <section className="notification-settings-section">
        <div className="notification-settings-heading">
          <div>
            <p className="label">
              IN-GAME NOTIFICATIONS
            </p>

            <h2>
              Overlay settings
            </h2>
          </div>
        </div>

        <div className="notification-settings-grid">
          <label className="notification-setting-row">
            <span>Enabled</span>
            <button
              className={`notification-toggle ${
                notificationSettings.enabled
                  ? "active"
                  : ""
              }`}
              type="button"
              role="switch"
              aria-checked={notificationSettings.enabled}
              disabled={notificationSettingsSaving}
              onClick={() =>
                void updateNotificationSettings({
                  ...notificationSettings,
                  enabled: !notificationSettings.enabled,
                })
              }
            >
              {notificationSettings.enabled ? "ON" : "OFF"}
            </button>
          </label>

          <label className="notification-setting-row">
            <span>Position</span>
            <select
              value={notificationSettings.position}
              disabled={notificationSettingsSaving}
              onChange={(event) =>
                void updateNotificationSettings({
                  ...notificationSettings,
                  position: event.target.value as NotificationPosition,
                })
              }
            >
              <option value="topLeft">Top Left</option>
              <option value="topRight">Top Right</option>
              <option value="bottomLeft">Bottom Left</option>
              <option value="bottomRight">Bottom Right</option>
            </select>
          </label>

          <label className="notification-setting-row">
            <span>Duration</span>
            <select
              value={notificationSettings.durationMs}
              disabled={notificationSettingsSaving}
              onChange={(event) =>
                void updateNotificationSettings({
                  ...notificationSettings,
                  durationMs: Number(event.target.value),
                })
              }
            >
              <option value={1000}>1.0 s</option>
              <option value={1500}>1.5 s</option>
              <option value={2000}>2.0 s</option>
              <option value={3000}>3.0 s</option>
            </select>
          </label>
        </div>
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
        Native file notifications with a lightweight
        100 ms safety check.
      </footer>
    </main>
  );
}

export default App;
