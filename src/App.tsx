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

  savestateCfgExists: boolean;
  prepareCfgExists: boolean;
  autoexecExists: boolean;

  savestateCfgValid: boolean;
  prepareCfgValid: boolean;
  autoexecValid: boolean;

  integrationHealthy: boolean;

  hotkeysRunning: boolean;
  hotkeysError: string | null;

  consoleWatcherRunning: boolean;
  consoleWatcherError: string | null;

  teleportsReady: boolean;
  presentationMaskActive: boolean;

  cameraRuntimeChecked: boolean;
  cameraRuntimeReady: boolean;
  cameraRuntimeError: string | null;

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

type SlotMetadata = {
  name: string;
  savedAt: number | null;
  color: string | null;
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

type SlotEditResult = {
  preset: number;
  slots: Array<PositionSnapshot | null>;
  historyState: HistoryState;
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

const SLOT_COLORS = [
  {
    label: "None",
    value: null,
  },
  {
    label: "Cyan",
    value: "#4fd1c5",
  },
  {
    label: "Yellow",
    value: "#ffd166",
  },
  {
    label: "Red",
    value: "#d98c8c",
  },
  {
    label: "Purple",
    value: "#9b8cff",
  },
  {
    label: "Green",
    value: "#62ff8f",
  },
] as const;

const CLEAR_PRESET_CONFIRMATION_KEY =
  "split.clearPreset.skipConfirmation";


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

  savestateCfgExists: false,
  prepareCfgExists: false,
  autoexecExists: false,

  savestateCfgValid: false,
  prepareCfgValid: false,
  autoexecValid: false,

  integrationHealthy: false,

  hotkeysRunning: false,
  hotkeysError: null,

  consoleWatcherRunning: false,
  consoleWatcherError: null,

  teleportsReady: false,
  presentationMaskActive: false,

  cameraRuntimeChecked: false,
  cameraRuntimeReady: false,
  cameraRuntimeError: null,

  source: "not-found",
};

type StatusTone =
  | "ok"
  | "warning"
  | "error"
  | "off";

function formatSavedAge(
  savedAt: number | null,
  nowMs: number,
): string | null {
  if (savedAt === null) {
    return null;
  }

  const ageSeconds = Math.max(
    0,
    Math.floor(nowMs / 1000) - savedAt,
  );

  if (ageSeconds < 60) {
    return "Saved just now";
  }

  const minutes =
    Math.floor(ageSeconds / 60);

  if (minutes < 60) {
    return `Saved ${minutes} min ago`;
  }

  const hours =
    Math.floor(minutes / 60);

  if (hours < 24) {
    return `Saved ${hours} h ago`;
  }

  const days =
    Math.floor(hours / 24);

  if (days < 7) {
    return `Saved ${days} d ago`;
  }

  return `Saved ${new Date(
    savedAt * 1000,
  ).toLocaleDateString()}`;
}  

function StatusDot({
  ok = false,
  tone,
}: {
  ok?: boolean;
  tone?: StatusTone;
}) {
  const resolvedTone =
    tone ?? (ok ? "ok" : "off");

  return (
    <span
      className={`status-dot ${resolvedTone}`}
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
    slotMetadata,
    setSlotMetadata,
  ] = useState<Array<SlotMetadata>>(
    () =>
      Array.from(
        { length: 8 },
        (_, index) => ({
          name: `Slot ${index + 1}`,
          savedAt: null,
          color: null,
        }),
      ),
  );

  const [
    relativeTimeNow,
    setRelativeTimeNow,
  ] = useState(() => Date.now());



  const [
    presetNames,
    setPresetNames,
  ] = useState<Array<string>>(
    () =>
      Array.from(
        { length: 4 },
        (_, index) =>
          `Preset ${index + 1}`,
      ),
  );

  const [
    renamingPreset,
    setRenamingPreset,
  ] = useState(false);

  const [
    clearingPreset,
    setClearingPreset,
  ] = useState(false);

  const [
    pendingClearPreset,
    setPendingClearPreset,
  ] = useState<{
    preset: number;
    name: string;
  } | null>(null);

  const [
    dontAskClearPresetAgain,
    setDontAskClearPresetAgain,
  ] = useState(false);

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
    coloringSlot,
    setColoringSlot,
  ] = useState<number | null>(
    null,
  );

  const [
    loading,
    setLoading,
  ] = useState(false);


  const [
    repairingIntegration,
    setRepairingIntegration,
  ] = useState(false);

  const [
    cameraRetrying,
    setCameraRetrying,
  ] = useState(false);

  const [
    watcherRetrying,
    setWatcherRetrying,
  ] = useState(false);

  const [
    teleportPreparing,
    setTeleportPreparing,
  ] = useState(false);

  const [
    presentationResuming,
    setPresentationResuming,
  ] = useState(false);

  const [
    diagnosticCopying,
    setDiagnosticCopying,
  ] = useState(false);

  const [
    diagnosticCopied,
    setDiagnosticCopied,
  ] = useState(false);

  const [
    error,
    setError,
  ] =
    useState<string | null>(null);
    


  useEffect(() => {
    const hasTimestamp =
      slotMetadata.some(
        (entry) =>
          entry.savedAt !== null,
      );

    if (!hasTimestamp) {
      return;
    }

    setRelativeTimeNow(
      Date.now(),
    );

    const timer =
      window.setInterval(
        () => {
          setRelativeTimeNow(
            Date.now(),
          );
        },
        30_000,
      );

    return () => {
      window.clearInterval(
        timer,
      );
    };
  }, [slotMetadata]);


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


  const repairIntegration =
    useCallback(async () => {
      setRepairingIntegration(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockStatus>(
            "repair_deadlock_integration",
          );

        setStatus(next);
      } catch (reason) {
        setError(String(reason));
      } finally {
        setRepairingIntegration(false);
      }
    }, []);
    
  const retryCamera =
    useCallback(async () => {
      setCameraRetrying(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockStatus>(
            "retry_camera_runtime",
          );

        setStatus(next);
      } catch (reason) {
        setError(String(reason));
      } finally {
        setCameraRetrying(false);
      }
    }, []);
    
  const retryConsoleWatcher =
    useCallback(async () => {
      setWatcherRetrying(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockStatus>(
            "retry_console_watcher",
          );

        setStatus(next);
      } catch (reason) {
        setError(String(reason));
      } finally {
        setWatcherRetrying(false);
      }
    }, []);
    
  const copyDiagnosticReport =
    useCallback(async () => {
      setDiagnosticCopying(true);
      setDiagnosticCopied(false);
      setError(null);

      try {
        const report =
          await invoke<string>(
            "get_diagnostic_report",
          );

        await navigator.clipboard.writeText(
          report,
        );

        setDiagnosticCopied(true);

        window.setTimeout(() => {
          setDiagnosticCopied(false);
        }, 2000);
      } catch (reason) {
        setError(
          `Could not copy diagnostic report: ${String(reason)}`,
        );
      } finally {
        setDiagnosticCopying(false);
      }
    }, []);  



  const prepareTeleportsNow =
    useCallback(async () => {
      setTeleportPreparing(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockStatus>(
            "prepare_teleports_now",
          );

        setStatus(next);
      } catch (reason) {
        setError(
          `Could not prepare teleport points: ${String(reason)}`,
        );
      } finally {
        setTeleportPreparing(false);
      }
    }, []);  

  const resumePresentation =
    useCallback(async () => {
      setPresentationResuming(true);
      setError(null);

      try {
        const next =
          await invoke<DeadlockStatus>(
            "resume_deadlock_presentation",
          );

        setStatus(next);
      } catch (reason) {
        setError(
          `Could not resume Deadlock presentation: ${String(reason)}`,
        );
      } finally {
        setPresentationResuming(false);
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
          metadata,
          preset,
          names,
          history,
          favoriteActive,
        ] =
          await Promise.all([
            invoke<
              Array<PositionSnapshot | null>
            >(
              "get_slots",
            ),

            invoke<
              Array<SlotMetadata>
            >(
              "get_slot_metadata",
            ),

            invoke<number>(
              "get_active_preset",
            ),

            invoke<Array<string>>(
              "get_preset_names",
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

          setSlotMetadata(
            metadata,
          );

          setActivePreset(
            preset,
          );

          setPresetNames(
            names,
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
        if (disposed) {
          return;
        }

        setSlots(
          event.payload,
        );

        setSavingSlot(
          null,
        );

        void invoke<
          Array<SlotMetadata>
        >(
          "get_slot_metadata",
        )
          .then((metadata) => {
            if (!disposed) {
              setSlotMetadata(
                metadata,
              );

              setRelativeTimeNow(
                Date.now(),
              );
            }
          })
          .catch((reason) => {
            console.error(
              "[SPLIT UI] Failed to refresh slot metadata:",
              reason,
            );
          });
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


      const applySlotEditResult =
        useCallback(
          async (
            result: SlotEditResult,
          ) => {
            const metadata =
              await invoke<
                Array<SlotMetadata>
              >(
                "get_slot_metadata",
              );

            setActivePreset(
              result.preset,
            );

            setSlots(
              result.slots,
            );

            setSlotMetadata(
              metadata,
            );

            setRelativeTimeNow(
              Date.now(),
            );

            setHistoryState(
              result.historyState,
            );

            setFavoriteMode(
              result.favoriteActive,
            );
          },
          [],
        );

      const renameSavedSlot =
        useCallback(
          async (
            slot: number,
            currentName: string,
          ) => {
            const name =
              window.prompt(
                `Rename slot ${slot}`,
                currentName,
              );

            if (name === null) {
              return;
            }

            const trimmed =
              name.trim();

            if (!trimmed) {
              setError(
                "Slot name cannot be empty",
              );

              return;
            }

            setError(null);

            try {
              const result =
                await invoke<SlotEditResult>(
                  "rename_slot",
                  {
                    slot,
                    name: trimmed,
                  },
                );

              await applySlotEditResult(
                result,
              );
            } catch (reason) {
              setError(
                String(reason),
              );
            }
          },
          [applySlotEditResult],
        );

      const clearSavedSlot =
        useCallback(
          async (
            slot: number,
            currentName: string,
          ) => {
            const confirmed =
              window.confirm(
                `Clear "${currentName}"?\n\n` +
                "The saved position, name, timestamp and color will be reset.",
              );

            if (!confirmed) {
              return;
            }

            setError(null);

            try {
              const result =
                await invoke<SlotEditResult>(
                  "clear_slot",
                  {
                    slot,
                  },
                );

              await applySlotEditResult(
                result,
              );
            } catch (reason) {
              setError(
                String(reason),
              );
            }
          },
          [applySlotEditResult],
        );



      const updateSlotColor =
        useCallback(
          async (
            slot: number,
            color: string | null,
          ) => {
            setColoringSlot(
              slot,
            );

            setError(
              null,
            );

            try {
              const result =
                await invoke<SlotEditResult>(
                  "set_slot_color",
                  {
                    slot,
                    color,
                  },
                );

              await applySlotEditResult(
                result,
              );
            } catch (reason) {
              setError(
                String(reason),
              );
            } finally {
              setColoringSlot(
                null,
              );
            }
          },
          [applySlotEditResult],
        );  


      const renameActivePreset =
        useCallback(
          async () => {
            const currentName =
              presetNames[
                activePreset - 1
              ] ??
              `Preset ${activePreset}`;

            const name =
              window.prompt(
                `Rename preset ${activePreset}`,
                currentName,
              );

            if (name === null) {
              return;
            }

            const trimmed =
              name.trim();

            if (!trimmed) {
              setError(
                "Preset name cannot be empty",
              );

              return;
            }

            setRenamingPreset(true);
            setError(null);

            try {
              const names =
                await invoke<
                  Array<string>
                >(
                  "rename_preset",
                  {
                    preset:
                      activePreset,
                    name:
                      trimmed,
                  },
                );

              setPresetNames(
                names,
              );
            } catch (reason) {
              setError(
                String(reason),
              );
            } finally {
              setRenamingPreset(
                false,
              );
            }
          },
          [
            activePreset,
            presetNames,
          ],
        );  


      const performClearPreset =
        useCallback(
          async (
            preset: number,
          ) => {
            setClearingPreset(true);
            setError(null);

            try {
              const result =
                await invoke<SlotEditResult>(
                  "clear_preset",
                  {
                    preset,
                  },
                );

              await applySlotEditResult(
                result,
              );

              const names =
                await invoke<
                  Array<string>
                >(
                  "get_preset_names",
                );

              setPresetNames(
                names,
              );
            } catch (reason) {
              setError(
                String(reason),
              );
            } finally {
              setClearingPreset(
                false,
              );
            }
          },
          [applySlotEditResult],
        );

      const clearActivePreset =
        useCallback(
          async () => {
            const presetName =
              presetNames[
                activePreset - 1
              ] ??
              `Preset ${activePreset}`;

            const skipConfirmation =
              localStorage.getItem(
                CLEAR_PRESET_CONFIRMATION_KEY,
              ) === "true";

            if (skipConfirmation) {
              await performClearPreset(
                activePreset,
              );

              return;
            }

            setDontAskClearPresetAgain(
              false,
            );

            setPendingClearPreset({
              preset:
                activePreset,
              name:
                presetName,
            });
          },
          [
            activePreset,
            presetNames,
            performClearPreset,
          ],
        );

      const confirmClearPreset =
        useCallback(
          async () => {
            const target =
              pendingClearPreset;

            if (!target) {
              return;
            }

            if (
              dontAskClearPresetAgain
            ) {
              localStorage.setItem(
                CLEAR_PRESET_CONFIRMATION_KEY,
                "true",
              );
            }

            setPendingClearPreset(
              null,
            );

            await performClearPreset(
              target.preset,
            );
          },
          [
            pendingClearPreset,
            dontAskClearPresetAgain,
            performClearPreset,
          ],
        );

      const cancelClearPreset =
        useCallback(
          () => {
            setPendingClearPreset(
              null,
            );

            setDontAskClearPresetAgain(
              false,
            );
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

        const metadata =
          await invoke<
            Array<SlotMetadata>
          >(
            "get_slot_metadata",
          );  


        setActivePreset(
          preset,
        );

        setSlots(
          saved,
        );

        setSlotMetadata(
          metadata,
        );

        setRelativeTimeNow(
          Date.now(),
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

          const metadata =
            await invoke<
              Array<SlotMetadata>
            >(
              "get_slot_metadata",
            );

          setActivePreset(
            result.preset,
          );

          setSlots(
            result.slots,
          );

          setSlotMetadata(
            metadata,
          );

          setRelativeTimeNow(
            Date.now(),
          );

          setHistoryState(
            result.historyState,
          );

          setFavoriteMode(
            result.favoriteActive,
          );
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


        const metadata =
          await invoke<
            Array<SlotMetadata>
          >(
            "get_slot_metadata",
          );  
        setActivePreset(result.preset);
        setSlots(result.slots);
        setSlotMetadata(
          metadata,
        );

        setRelativeTimeNow(
          Date.now(),
        );
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
  * Résumé global du Health Check.
  *
  * ERROR =
  * fonctionnalité SPLIT réellement cassée.
  *
  * WARNING =
  * état temporaire / action utilisateur
  * potentiellement nécessaire.
  */
  const healthIssueCount = [
    !status.integrationHealthy,
    !status.hotkeysRunning,
    !status.consoleWatcherRunning,
    status.presentationMaskActive,
    status.cameraRuntimeChecked &&
      !status.cameraRuntimeReady,
  ].filter(Boolean).length;

  const healthWarningCount = [
    !status.deadlockRunning,

    status.deadlockRunning &&
      !status.teleportsReady,

    status.deadlockRunning &&
      !status.cameraRuntimeChecked,

    status.deadlockRunning &&
      !status.consoleLogExists,
  ].filter(Boolean).length;

  const healthTone: StatusTone =
    healthIssueCount > 0
      ? "error"
      : healthWarningCount > 0
        ? "warning"
        : "ok";

  const healthHeadline =
    healthIssueCount > 0
      ? "Attention required"
      : healthWarningCount > 0
        ? "Operational with warnings"
        : "All systems operational";

  const issueText =
    `${healthIssueCount} ${
      healthIssueCount === 1
        ? "issue"
        : "issues"
    }`;

  const warningText =
    `${healthWarningCount} ${
      healthWarningCount === 1
        ? "warning"
        : "warnings"
    }`;

  const healthDescription =
    healthIssueCount > 0
      ? `${issueText} ${
          healthIssueCount === 1
            ? "requires"
            : "require"
        } attention${
          healthWarningCount > 0
            ? ` · ${warningText}`
            : ""
        }.`
      : healthWarningCount > 0
        ? `No critical issues · ${warningText}.`
        : "All monitored SPLIT systems are ready.";

  return (
    <main className="shell">
      {pendingClearPreset && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="clear-preset-title"
          >
            <div className="confirmation-content">
              <span
                className="confirmation-warning"
                aria-hidden="true"
              >
                !
              </span>

              <div>
                <h3
                  id="clear-preset-title"
                  className="confirmation-title"
                >
                  Clear preset
                </h3>

                <p className="confirmation-message">
                  Are you sure you want to clear{" "}
                  <strong>
                    &quot;
                    {pendingClearPreset.name}
                    &quot;
                  </strong>
                  ?
                </p>

                <p className="confirmation-description">
                  This will erase all 8 slots
                  and reset the preset name.
                </p>
              </div>
            </div>

            <label className="confirmation-checkbox">
              <input
                type="checkbox"
                checked={
                  dontAskClearPresetAgain
                }
                onChange={(event) =>
                  setDontAskClearPresetAgain(
                    event.target.checked,
                  )
                }
              />

              <span>
                Don't ask me again
              </span>
            </label>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                onClick={
                  cancelClearPreset
                }
              >
                Cancel
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={() =>
                  void confirmClearPreset()
                }
              >
                Clear preset
              </button>
            </div>
          </section>
        </div>
      )}

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

        <div className="topbar-actions">
          <button
            className="refresh-button"
            type="button"
            onClick={() =>
              void copyDiagnosticReport()
            }
            disabled={diagnosticCopying}
          >
            {diagnosticCopying
              ? "Copying…"
              : diagnosticCopied
                ? "Copied!"
                : "Copy diagnostic"}
          </button>

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
        </div>
      </header>

      <section
        className={`hero-card health-summary ${healthTone}`}
      >
        <div className="hero-status">
          <StatusDot
            tone={healthTone}
          />

          <div>
            <span className="label">
              SYSTEM HEALTH
            </span>

            <strong>
              {healthHeadline}
            </strong>

            <p className="health-summary-description">
              {healthDescription}
            </p>
          </div>
        </div>

        <div className="health-summary-counts">
          {healthIssueCount > 0 && (
            <span className="health-count error">
              {issueText}
            </span>
          )}

          {healthWarningCount > 0 && (
            <span className="health-count warning">
              {warningText}
            </span>
          )}

          {healthIssueCount === 0 &&
            healthWarningCount === 0 && (
              <span className="health-count ok">
                All clear
              </span>
            )}
        </div>
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
              {presetNames[
                preset - 1
              ] ??
                `Preset ${preset}`}
            </button>
          ),
        )}
      </div>

      <div className="preset-management">
        <button
          className="preset-button"
          type="button"
          disabled={
            favoriteMode ||
            renamingPreset ||
            clearingPreset ||
            savingSlot !== null ||
            loadingSlot !== null ||
            coloringSlot !== null
          }
          onClick={() =>
            void renameActivePreset()
          }
        >
          {renamingPreset
            ? "Renaming…"
            : "Rename preset"}
        </button>

        <button
          className="preset-button preset-clear-button"
          type="button"
          disabled={
            favoriteMode ||
            renamingPreset ||
            clearingPreset ||
            savingSlot !== null ||
            loadingSlot !== null ||
            coloringSlot !== null
          }
          onClick={() =>
            void clearActivePreset()
          }
        >
          {clearingPreset
            ? "Clearing…"
            : "Clear preset"}
        </button>
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

            const metadata =
              slotMetadata[index];

            const displayName =
              metadata?.name?.trim() ||
              (
                favoriteMode
                  ? `Favorite ${slot}`
                  : `Slot ${slot}`
              );


            const defaultName =
              favoriteMode
                ? `Favorite ${slot}`
                : `Slot ${slot}`;

            const canClear =
              position !== null ||
              displayName !== defaultName;  

            const savedAge =
              position
                ? formatSavedAge(
                    metadata?.savedAt ?? null,
                    relativeTimeNow,
                  )
                : null;

            return (
              <article
                className="slot-card"
                key={slot}
              >
                <div className="slot-top">
                  <div className="slot-title">
                    <div className="slot-name-row">
                      {metadata?.color && (
                        <span
                          className="slot-color-indicator"
                          style={{
                            backgroundColor:
                              metadata.color,
                          }}
                        />
                      )}

                      <span className="slot-number">
                        {displayName}
                      </span>
                    </div>

                    {savedAge && (
                      <span className="slot-saved-age">
                        {savedAge}
                      </span>
                    )}
                  </div>

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

            <div className="slot-color-picker">
              {SLOT_COLORS.map(
                ({ label, value }) => {
                  const active =
                    metadata?.color === value;

                  return (
                    <button
                      className={`slot-color-button ${
                        active ? "active" : ""
                      }`}
                      type="button"
                      key={label}
                      title={label}
                      aria-label={`${label} slot color`}
                      disabled={
                        !position ||
                        savingSlot !== null ||
                        loadingSlot !== null ||
                        coloringSlot !== null
                      }
                      onClick={() =>
                        void updateSlotColor(
                          slot,
                          value,
                        )
                      }
                    >
                      {value ? (
                        <span
                          className="slot-color-swatch"
                          style={{
                            backgroundColor:
                              value,
                          }}
                        />
                      ) : (
                        <span className="slot-color-none">
                          ×
                        </span>
                      )}
                    </button>
                  );
                },
              )}
            </div>


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

              <button
                className="slot-save-button slot-rename-button"
                type="button"
                disabled={
                  savingSlot !== null ||
                  loadingSlot !== null
                }
                onClick={() =>
                  void renameSavedSlot(
                    slot,
                    displayName,
                  )
                }
              >
                Rename
              </button>

              <button
                className="slot-save-button slot-clear-button"
                type="button"
                disabled={
                  !canClear ||
                  savingSlot !== null ||
                  loadingSlot !== null
                }
                onClick={() =>
                  void clearSavedSlot(
                    slot,
                    displayName,
                  )
                }
              >
                Clear
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
        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot
              ok={status.integrationHealthy}
            />

            <span>
              SPLIT integration
            </span>
          </div>

          <strong>
            {status.integrationHealthy
              ? "Healthy"
              : "Needs attention"}
          </strong>

          {!status.integrationHealthy && (
            <button
              className="refresh-button"
              type="button"
              disabled={repairingIntegration}
              onClick={() =>
                void repairIntegration()
              }
            >
              {repairingIntegration
                ? "Repairing…"
                : "Repair integration"}
            </button>
          )}
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={status.deadlockRunning}
            />

            <span>
              Deadlock process
            </span>
          </div>

          <strong>
            {status.deadlockRunning
              ? "Running"
              : "Not running"}
          </strong>
        </article>

        <article
          className={`status-card ${
            status.hotkeysRunning
              ? ""
              : "wide diagnostic-card"
          }`}
        >
          <div className="status-heading">
            <StatusDot
              tone={
                status.hotkeysRunning
                  ? "ok"
                  : "error"
              }
            />

            <span>
              Hotkey hook
            </span>
          </div>

          <strong>
            {status.hotkeysRunning
              ? "Running"
              : "Down"}
          </strong>

          {!status.hotkeysRunning && (
            <div className="diagnostic-details">
              <div className="diagnostic-reason">
                <span>
                  REASON
                </span>

                <code>
                  {status.hotkeysError ??
                    "The Windows keyboard hook is not running."}
                </code>
              </div>

              <div className="diagnostic-fix">
                <span>
                  HOW TO FIX
                </span>

                <ol>
                  <li>
                    Close every other running
                    instance of SPLIT.
                  </li>

                  <li>
                    Completely restart SPLIT.
                  </li>

                  <li>
                    If Deadlock is running as
                    administrator, run SPLIT with
                    the same privilege level.
                  </li>

                  <li>
                    Temporarily disable software
                    that intercepts global keyboard
                    input and test again.
                  </li>

                  <li>
                    If the issue persists, copy the
                    diagnostic report before
                    reporting the problem.
                  </li>
                </ol>
              </div>

              <p className="diagnostic-description">
                Hotkeys cannot currently be restarted
                safely inside the same SPLIT process.
                Restarting SPLIT is required.
              </p>
            </div>
          )}
        </article>

        <article
          className={`status-card ${
            status.consoleWatcherRunning
              ? ""
              : "wide diagnostic-card"
          }`}
        >
          <div className="status-heading">
            <StatusDot
              tone={
                status.consoleWatcherRunning
                  ? "ok"
                  : "error"
              }
            />

            <span>
              Console watcher
            </span>
          </div>

          <strong>
            {status.consoleWatcherRunning
              ? "Running"
              : "Down"}
          </strong>

          {!status.consoleWatcherRunning && (
            <div className="diagnostic-details">
              <div className="diagnostic-reason">
                <span>
                  REASON
                </span>

                <code>
                  {status.consoleWatcherError ??
                    "The console watcher is not running."}
                </code>
              </div>

              <div className="diagnostic-fix">
                <span>
                  HOW TO FIX
                </span>

                <ol>
                  <li>
                    Make sure SPLIT points to the
                    correct Deadlock installation.
                  </li>

                  <li>
                    Check that the
                    {" "}
                    <code>game\citadel</code>
                    {" "}
                    folder still exists.
                  </li>

                  <li>
                    If console.log is missing,
                    launch Deadlock once.
                  </li>

                  <li>
                    Click Retry watcher below.
                  </li>

                  <li>
                    If it still fails, check Windows
                    permissions or antivirus software
                    blocking SPLIT.
                  </li>
                </ol>
              </div>

              <div className="diagnostic-actions">
                <button
                  className="refresh-button"
                  type="button"
                  disabled={watcherRetrying}
                  onClick={() =>
                    void retryConsoleWatcher()
                  }
                >
                  {watcherRetrying
                    ? "Restarting…"
                    : "Retry watcher"}
                </button>
              </div>
            </div>
          )}
        </article>

        <article
          className={`status-card ${
            status.teleportsReady
              ? ""
              : "wide diagnostic-card"
          }`}
        >
          <div className="status-heading">
            <StatusDot
              tone={
                status.teleportsReady
                  ? "ok"
                  : "warning"
              }
            />

            <span>
              Teleport preparation
            </span>
          </div>

          <strong>
            {status.teleportsReady
              ? "Ready"
              : "Pending"}
          </strong>

          {!status.teleportsReady && (
            <div className="diagnostic-details">
              <p className="diagnostic-description">
                SPLIT generated a new set of
                teleport points, but Deadlock has
                not prepared them yet.
              </p>

              <p className="diagnostic-description">
                This is usually normal after
                startup repair, saving a slot,
                switching preset, or changing the
                active slot bank.
              </p>

              <div className="diagnostic-fix">
                <span>
                  HOW TO FIX
                </span>

                <ol>
                  <li>
                    Make sure Deadlock is running.
                  </li>

                  <li>
                    Enter Sandbox or Practice mode.
                  </li>

                  <li>
                    Click Prepare now below.
                  </li>

                  <li>
                    Alternatively, loading any
                    populated slot will prepare the
                    teleport points automatically.
                  </li>
                </ol>
              </div>

              <div className="diagnostic-actions">
                <button
                  className="refresh-button"
                  type="button"
                  disabled={
                    teleportPreparing ||
                    !status.deadlockRunning
                  }
                  onClick={() =>
                    void prepareTeleportsNow()
                  }
                >
                  {teleportPreparing
                    ? "Preparing…"
                    : "Prepare now"}
                </button>

                {!status.deadlockRunning && (
                  <span className="diagnostic-action-hint">
                    Start Deadlock first.
                  </span>
                )}
              </div>
            </div>
          )}
        </article>

        <article
          className={`status-card ${
            status.presentationMaskActive
              ? "wide diagnostic-card"
              : ""
          }`}
        >
          <div className="status-heading">
            <StatusDot
              tone={
                status.presentationMaskActive
                  ? "error"
                  : "ok"
              }
            />

            <span>
              Presentation mask
            </span>
          </div>

          <strong>
            {status.presentationMaskActive
              ? "Active"
              : "Normal"}
          </strong>

          {status.presentationMaskActive && (
            <div className="diagnostic-details">
              <p className="diagnostic-description">
                SPLIT believes Deadlock presentation
                is still paused by
                r_force_no_present.
              </p>

              <p className="diagnostic-description">
                Deadlock may appear frozen even
                though the game process is still
                running normally.
              </p>

              <div className="diagnostic-fix">
                <span>
                  HOW TO FIX
                </span>

                <ol>
                  <li>
                    Click Resume presentation below.
                  </li>

                  <li>
                    If automatic recovery fails,
                    bring Deadlock to the foreground.
                  </li>

                  <li>
                    Press F10 manually. During an
                    active presentation mask, F10 is
                    SPLIT's emergency recovery key.
                  </li>

                  <li>
                    If this happens repeatedly, copy
                    the diagnostic report and report
                    the issue.
                  </li>
                </ol>
              </div>

              <div className="diagnostic-actions">
                <button
                  className="refresh-button"
                  type="button"
                  disabled={
                    presentationResuming ||
                    !status.deadlockRunning
                  }
                  onClick={() =>
                    void resumePresentation()
                  }
                >
                  {presentationResuming
                    ? "Resuming…"
                    : "Resume presentation"}
                </button>

                {!status.deadlockRunning && (
                  <span className="diagnostic-action-hint">
                    Deadlock is not running.
                  </span>
                )}
              </div>
            </div>
          )}
        </article>

        

        <article className="status-card wide diagnostic-card">
          <div className="status-heading">
            <StatusDot
              tone={
                !status.cameraRuntimeChecked
                  ? "warning"
                  : status.cameraRuntimeReady
                    ? "ok"
                    : "error"
              }
            />

            <span>
              Camera runtime
            </span>
          </div>

          <strong>
            {!status.cameraRuntimeChecked
              ? "Not tested"
              : status.cameraRuntimeReady
                ? "Ready"
                : "Unavailable"}
          </strong>

          {!status.cameraRuntimeReady && (
            <div className="diagnostic-details">
              {!status.cameraRuntimeChecked ? (
                <>
                  <p className="diagnostic-description">
                    The camera runtime has not been
                    checked during this Deadlock
                    session yet.
                  </p>

                  <p className="diagnostic-description">
                    Start Deadlock and enter
                    Sandbox / Practice mode, then
                    test the camera.
                  </p>
                </>
              ) : (
                <>
                  <div className="diagnostic-reason">
                    <span>
                      REASON
                    </span>

                    <code>
                      {status.cameraRuntimeError ??
                        "Unknown camera runtime error"}
                    </code>
                  </div>

                  <div className="diagnostic-fix">
                    <span>
                      HOW TO FIX
                    </span>

                    <ol>
                      <li>
                        Make sure Deadlock is running.
                      </li>

                      <li>
                        Enter Sandbox or Practice mode
                        so the in-game camera is active.
                      </li>

                      <li>
                        Click Retry camera below.
                      </li>

                      <li>
                        If the problem started after a
                        Deadlock update, update SPLIT
                        to the latest version.
                      </li>
                    </ol>
                  </div>
                </>
              )}

              <div className="diagnostic-actions">
                <button
                  className="refresh-button"
                  type="button"
                  disabled={
                    cameraRetrying ||
                    !status.deadlockRunning
                  }
                  onClick={() =>
                    void retryCamera()
                  }
                >
                  {cameraRetrying
                    ? "Testing…"
                    : status.cameraRuntimeChecked
                      ? "Retry camera"
                      : "Test camera"}
                </button>

                {!status.deadlockRunning && (
                  <span className="diagnostic-action-hint">
                    Start Deadlock first.
                  </span>
                )}
              </div>
            </div>
          )}
        </article>    

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={Boolean(status.deadlockPath)}
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
              ok={status.cfgDirExists}
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

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={status.savestateCfgValid}
            />

            <span>
              savestate.cfg
            </span>
          </div>

          <strong>
            {status.savestateCfgValid
              ? "Valid"
              : status.savestateCfgExists
                ? "Invalid"
                : "Missing"}
          </strong>
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={status.prepareCfgValid}
            />

            <span>
              savestate_prepare.cfg
            </span>
          </div>

          <strong>
            {status.prepareCfgValid
              ? "Valid"
              : status.prepareCfgExists
                ? "Invalid"
                : "Missing"}
          </strong>
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={status.autoexecValid}
            />

            <span>
              autoexec.cfg
            </span>
          </div>

          <strong>
            {status.autoexecValid
              ? "Configured"
              : status.autoexecExists
                ? "Missing SPLIT entry"
                : "Missing"}
          </strong>
        </article>

        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot
              ok={status.consoleLogExists}
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
              ok={Boolean(lastPosition)}
            />

            <span>
              Last parsed position
            </span>
          </div>

          {lastPosition ? (
            <code>
              XYZ {lastPosition.x}{" "}
              {lastPosition.y}{" "}
              {lastPosition.z} · ANG{" "}
              {lastPosition.pitch}{" "}
              {lastPosition.yaw}{" "}
              {lastPosition.roll}
            </code>
          ) : (
            <strong>
              Waiting for a new getpos response…
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
