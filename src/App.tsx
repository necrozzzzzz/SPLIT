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
  save,
} from "@tauri-apps/plugin-dialog";

import {
  readTextFile,
  writeTextFile,
} from "@tauri-apps/plugin-fs";

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

type FavoriteSlotSummary = {
  slot: number;
  occupied: boolean;
  name: string;
  savedAt: number | null;
  color: string | null;
};

type SlotMetadataExport = SlotMetadata & {
  snapshot: PositionSnapshot | null;
};

type PresetExport = {
  format: "split-preset";
  version: number;
  name: string;
  slots: Array<SlotMetadataExport>;
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

type Hotkey = {
  key: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
};

type HotkeySettings = {
  loadSlots: Array<Hotkey>;
  saveSlots: Array<Hotkey>;
  undo: Hotkey;
  redo: Hotkey;
  cyclePreset: Hotkey;
  favoriteMode: Hotkey;
};

type HotkeyTarget =
  | { group: "loadSlots" | "saveSlots"; index: number }
  | { group: "undo" | "redo" | "cyclePreset" | "favoriteMode" };

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

const GAMEPLAY_HOTKEY_WARNING_KEY =
  "split.hotkeys.skipGameplayWarning";  

const FAVORITE_MODE_WARNING_KEY =
  "split.favorites.skipModeWarning";  


const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  enabled: true,
  position: "topRight",
  durationMs: 1500,
};

const DEFAULT_HOTKEY_SETTINGS: HotkeySettings = {
  loadSlots: Array.from({ length: 8 }, (_, index) => ({
    key: `F${index + 1}`,
    ctrl: false,
    alt: false,
    shift: false,
  })),
  saveSlots: Array.from({ length: 8 }, (_, index) => ({
    key: `F${index + 1}`,
    ctrl: false,
    alt: true,
    shift: false,
  })),
  undo: { key: "F9", ctrl: false, alt: false, shift: false },
  redo: { key: "F10", ctrl: false, alt: false, shift: false },
  cyclePreset: { key: "V", ctrl: false, alt: false, shift: false },
  favoriteMode: { key: "F11", ctrl: false, alt: false, shift: false },
};

function formatHotkey(hotkey: Hotkey): string {
  return [
    hotkey.ctrl ? "Ctrl" : null,
    hotkey.alt ? "Alt" : null,
    hotkey.shift ? "Shift" : null,
    hotkey.key,
  ].filter(Boolean).join(" + ");
}

function capturedKey(event: KeyboardEvent): string | null {
  if (/^[a-z0-9]$/i.test(event.key)) return event.key.toUpperCase();
  if (/^F(?:[1-9]|1[0-2])$/i.test(event.key)) return event.key.toUpperCase();
  const supported: Record<string, string> = {
    ArrowUp: "ArrowUp", ArrowDown: "ArrowDown", ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight", Home: "Home", End: "End", Insert: "Insert",
    Delete: "Delete", PageUp: "PageUp", PageDown: "PageDown", " ": "Space",
    Spacebar: "Space",
  };
  return supported[event.key] ?? null;
}

function isHotkeyTarget(active: HotkeyTarget | null, target: HotkeyTarget): boolean {
  return active?.group === target.group
    && (!("index" in target) || ("index" in active && active.index === target.index));
}

function hotkeysEqual(
  first: Hotkey,
  second: Hotkey,
): boolean {
  return (
    first.key === second.key &&
    first.ctrl === second.ctrl &&
    first.alt === second.alt &&
    first.shift === second.shift
  );
}

function findHotkeyConflictLabel(
  settings: HotkeySettings,
  target: HotkeyTarget,
  candidate: Hotkey,
): string | null {
  for (let index = 0; index < 8; index += 1) {
    if (
      !(
        target.group === "loadSlots" &&
        target.index === index
      ) &&
      hotkeysEqual(
        settings.loadSlots[index],
        candidate,
      )
    ) {
      return `Load Slot ${index + 1}`;
    }

    if (
      !(
        target.group === "saveSlots" &&
        target.index === index
      ) &&
      hotkeysEqual(
        settings.saveSlots[index],
        candidate,
      )
    ) {
      return `Save Slot ${index + 1}`;
    }
  }

  if (
    target.group !== "undo" &&
    hotkeysEqual(
      settings.undo,
      candidate,
    )
  ) {
    return "Undo";
  }

  if (
    target.group !== "redo" &&
    hotkeysEqual(
      settings.redo,
      candidate,
    )
  ) {
    return "Redo";
  }

  if (
    target.group !== "cyclePreset" &&
    hotkeysEqual(
      settings.cyclePreset,
      candidate,
    )
  ) {
    return "Cycle Preset";
  }

  if (
    target.group !== "favoriteMode" &&
    hotkeysEqual(
      settings.favoriteMode,
      candidate,
    )
  ) {
    return "Favorite Mode";
  }

  return null;
}

function hotkeyMayInterfereWithGameplay(hotkey: Hotkey): boolean {
  const usesModifier =
    hotkey.ctrl ||
    hotkey.alt ||
    hotkey.shift;

  if (!usesModifier) {
    return false;
  }

  const gameplayKey =
    /^[A-Z0-9]$/.test(hotkey.key) ||
    [
      "Space",
      "ArrowUp",
      "ArrowDown",
      "ArrowLeft",
      "ArrowRight",
    ].includes(hotkey.key);

  return gameplayKey;
}

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

type AppView = "slots" | "settings";

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
  const [activeView, setActiveView] =
    useState<AppView>("slots");

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
    exportingPreset,
    setExportingPreset,
  ] = useState(false);

  const [
    importingPreset,
    setImportingPreset,
  ] = useState(false);

  const [
    pendingImportPreset,
    setPendingImportPreset,
  ] = useState<{
    preset: number;
    currentName: string;
    importedName: string;
    imported: unknown;
  } | null>(null);

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
    favoriteSlotSummaries,
    setFavoriteSlotSummaries,
  ] = useState<Array<FavoriteSlotSummary>>([]);

  const [
    pendingFavoriteCopy,
    setPendingFavoriteCopy,
  ] = useState<{
    sourceSlot: number;
    sourceName: string;
  } | null>(null);

  const [
    pendingFavoriteOverwrite,
    setPendingFavoriteOverwrite,
  ] = useState<{
    sourceSlot: number;
    sourceName: string;
    target: FavoriteSlotSummary;
  } | null>(null);

  const [
    favoriteCopyWorking,
    setFavoriteCopyWorking,
  ] = useState(false);

  const [
    favoriteModeWarningOpen,
    setFavoriteModeWarningOpen,
  ] = useState(false);

  const [
    dontRemindFavoriteModeAgain,
    setDontRemindFavoriteModeAgain,
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

  const [hotkeySettings, setHotkeySettings] =
    useState<HotkeySettings>(DEFAULT_HOTKEY_SETTINGS);
  const [hotkeySettingsSaving, setHotkeySettingsSaving] =
    useState(false);
  const [capturingHotkey, setCapturingHotkey] =
    useState<HotkeyTarget | null>(null);
  const [hotkeyMessage, setHotkeyMessage] =
    useState<string | null>(null);
  const [hotkeysRestored, setHotkeysRestored] =
    useState(false);

  const [
    pendingGameplayHotkey,
    setPendingGameplayHotkey,
  ] = useState<{
    target: HotkeyTarget;
    hotkey: Hotkey;
  } | null>(null);

  const [
    dontRemindGameplayHotkeyAgain,
    setDontRemindGameplayHotkeyAgain,
  ] = useState(false);


  const [
    clearPresetConfirmationRestored,
    setClearPresetConfirmationRestored,
  ] = useState(false);

  const [
    gameplayHotkeyWarningRestored,
    setGameplayHotkeyWarningRestored,
  ] = useState(false);

  const [
    favoriteModeWarningRestored,
    setFavoriteModeWarningRestored,
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

      const exportActivePreset =
        useCallback(
          async () => {
            setExportingPreset(true);
            setError(null);

            try {
              const exported =
                await invoke<PresetExport>(
                  "export_preset",
                  {
                    preset:
                      activePreset,
                  },
                );
              const sanitizedName =
                exported.name
                  .replace(
                    /[<>:"/\\|?*\u0000-\u001f]/g,
                    "_",
                  )
                  .replace(/[. ]+$/g, "");
              const safeName =
                /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(
                  sanitizedName,
                )
                  ? `_${sanitizedName}`
                  : sanitizedName ||
                    `Preset ${activePreset}`;
              const filePath =
                await save({
                  defaultPath:
                    `${safeName}.split-preset.json`,
                  filters: [
                    {
                      name: "SPLIT preset",
                      extensions: ["json"],
                    },
                  ],
                });

              if (filePath === null) {
                return;
              }

              await writeTextFile(
                filePath,
                `${JSON.stringify(exported, null, 2)}\n`,
              );
            } catch (reason) {
              setError(String(reason));
            } finally {
              setExportingPreset(false);
            }
          },
          [activePreset],
        );

      const selectPresetImport =
        useCallback(
          async () => {
            setImportingPreset(true);
            setError(null);

            try {
              const filePath =
                await open({
                  multiple: false,
                  directory: false,
                  filters: [
                    {
                      name: "SPLIT preset",
                      extensions: [
                        "split-preset.json",
                        "json",
                      ],
                    },
                  ],
                });

              if (
                filePath === null ||
                Array.isArray(filePath)
              ) {
                return;
              }

              const imported: unknown =
                JSON.parse(
                  await readTextFile(filePath),
                );
              const importedName =
                typeof imported === "object" &&
                imported !== null &&
                "name" in imported &&
                typeof imported.name === "string"
                  ? imported.name
                  : "Invalid preset";
              const currentName =
                presetNames[
                  activePreset - 1
                ] ??
                `Preset ${activePreset}`;

              setPendingImportPreset({
                preset: activePreset,
                currentName,
                importedName,
                imported,
              });
            } catch (reason) {
              setError(String(reason));
            } finally {
              setImportingPreset(false);
            }
          },
          [activePreset, presetNames],
        );

      const confirmPresetImport =
        useCallback(
          async () => {
            const target =
              pendingImportPreset;

            if (!target) {
              return;
            }

            setPendingImportPreset(null);
            setImportingPreset(true);
            setError(null);

            try {
              const result =
                await invoke<SlotEditResult>(
                  "import_preset",
                  {
                    preset: target.preset,
                    imported: target.imported,
                  },
                );

              await applySlotEditResult(result);

              const names =
                await invoke<Array<string>>(
                  "get_preset_names",
                );
              setPresetNames(names);
            } catch (reason) {
              setError(String(reason));
            } finally {
              setImportingPreset(false);
            }
          },
          [
            applySlotEditResult,
            pendingImportPreset,
          ],
        );

      const cancelPresetImport =
        useCallback(() => {
          setPendingImportPreset(null);
        }, []);

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
    if (!favoriteMode) {
      setFavoriteModeWarningOpen(false);
      setDontRemindFavoriteModeAgain(false);

      return;
    }

    const skipWarning =
      localStorage.getItem(
        FAVORITE_MODE_WARNING_KEY,
      ) === "true";

    if (skipWarning) {
      return;
    }

    setDontRemindFavoriteModeAgain(false);
    setFavoriteModeWarningOpen(true);
  }, [favoriteMode]);

  const continueFavoriteMode =
    useCallback(() => {
      if (dontRemindFavoriteModeAgain) {
        localStorage.setItem(
          FAVORITE_MODE_WARNING_KEY,
          "true",
        );
      }

      setFavoriteModeWarningOpen(false);
      setDontRemindFavoriteModeAgain(false);
    }, [dontRemindFavoriteModeAgain]);

  const leaveFavoriteMode =
    useCallback(async () => {
      setFavoriteModeWarningOpen(false);
      setDontRemindFavoriteModeAgain(false);

      if (!favoriteMode) {
        return;
      }

      await toggleFavorites();
    }, [
      favoriteMode,
      toggleFavorites,
    ]);  


  const openSaveToFavorite =
    useCallback(
      async (
        sourceSlot: number,
        sourceName: string,
      ) => {
        setFavoriteCopyWorking(true);
        setError(null);

        try {
          const favorites =
            await invoke<Array<FavoriteSlotSummary>>(
              "get_favorite_slot_summaries",
            );

          setFavoriteSlotSummaries(
            favorites,
          );

          setPendingFavoriteCopy({
            sourceSlot,
            sourceName,
          });
        } catch (reason) {
          setError(
            `Could not load Favorites: ${String(reason)}`,
          );
        } finally {
          setFavoriteCopyWorking(false);
        }
      },
      [],
    );

  const performFavoriteCopy =
    useCallback(
      async (
        sourceSlot: number,
        favoriteSlot: number,
        overwrite: boolean,
      ) => {
        setFavoriteCopyWorking(true);
        setError(null);

        try {
          await invoke<FavoriteSlotSummary>(
            "copy_slot_to_favorite",
            {
              sourceSlot,
              favoriteSlot,
              overwrite,
            },
          );

          setPendingFavoriteCopy(null);
          setPendingFavoriteOverwrite(null);
        } catch (reason) {
          setError(
            `Could not save to Favorites: ${String(reason)}`,
          );
        } finally {
          setFavoriteCopyWorking(false);
        }
      },
      [],
    );

  const chooseFavoriteDestination =
    useCallback(
      async (
        target: FavoriteSlotSummary,
      ) => {
        const source =
          pendingFavoriteCopy;

        if (!source) {
          return;
        }

        if (target.occupied) {
          setPendingFavoriteOverwrite({
            sourceSlot:
              source.sourceSlot,
            sourceName:
              source.sourceName,
            target,
          });

          setPendingFavoriteCopy(null);

          return;
        }

        await performFavoriteCopy(
          source.sourceSlot,
          target.slot,
          false,
        );
      },
      [
        pendingFavoriteCopy,
        performFavoriteCopy,
      ],
    );

  const cancelFavoriteCopy =
    useCallback(() => {
      setPendingFavoriteCopy(null);
    }, []);

  const cancelFavoriteOverwrite =
    useCallback(() => {
      const pending =
        pendingFavoriteOverwrite;

      setPendingFavoriteOverwrite(null);

      if (!pending) {
        return;
      }

      setPendingFavoriteCopy({
        sourceSlot:
          pending.sourceSlot,
        sourceName:
          pending.sourceName,
      });
    }, [pendingFavoriteOverwrite]);

  const confirmFavoriteOverwrite =
    useCallback(async () => {
      const pending =
        pendingFavoriteOverwrite;

      if (!pending) {
        return;
      }

      await performFavoriteCopy(
        pending.sourceSlot,
        pending.target.slot,
        true,
      );
    }, [
      pendingFavoriteOverwrite,
      performFavoriteCopy,
    ]);  

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

  useEffect(() => {
    let disposed = false;
    invoke<HotkeySettings>("get_hotkey_settings")
      .then((saved) => {
        if (!disposed) setHotkeySettings(saved);
      })
      .catch((reason) => {
        if (!disposed) setError(String(reason));
      });
    return () => {
      disposed = true;
    };
  }, []);

  const saveCapturedHotkey = useCallback(
    async (target: HotkeyTarget, hotkey: Hotkey) => {
      const conflictLabel =
        findHotkeyConflictLabel(
          hotkeySettings,
          target,
          hotkey,
        );

      if (conflictLabel) {
        setHotkeyMessage(
          `${formatHotkey(hotkey)} is already assigned to ${conflictLabel}.`,
        );

        return;
      }
      const next: HotkeySettings = {
        ...hotkeySettings,
        loadSlots: [...hotkeySettings.loadSlots],
        saveSlots: [...hotkeySettings.saveSlots],
      };
      if (target.group === "loadSlots" || target.group === "saveSlots") {
        next[target.group][target.index] = hotkey;
      } else {
        next[target.group] = hotkey;
      }
      setHotkeySettingsSaving(true);
      setHotkeyMessage(null);
      try {
        const saved = await invoke<HotkeySettings>("update_hotkey_settings", {
          settings: next,
        });
        setHotkeySettings(saved);
        setCapturingHotkey(null);
      } catch (reason) {
        setHotkeyMessage(String(reason));
      } finally {
        setHotkeySettingsSaving(false);
      }
    },
    [hotkeySettings],
  );

  useEffect(() => {
    if (!capturingHotkey) return;
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat || hotkeySettingsSaving) return;
      if (event.key === "Escape") {
        setCapturingHotkey(null);
        setHotkeyMessage(null);
        return;
      }
      if (event.metaKey) {
        setHotkeyMessage("Win/Meta hotkeys are not supported.");
        return;
      }
      if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) return;
      const key = capturedKey(event);
      if (!key) {
        setHotkeyMessage(`${event.key} is not a supported hotkey.`);
        return;
      }
      const capturedHotkey: Hotkey = {
        key,
        ctrl: event.ctrlKey,
        alt: event.altKey,
        shift: event.shiftKey,
      };

      if (hotkeyMayInterfereWithGameplay(capturedHotkey)) {
        const skipGameplayWarning =
          localStorage.getItem(
            GAMEPLAY_HOTKEY_WARNING_KEY,
          ) === "true";

        if (!skipGameplayWarning) {
          setDontRemindGameplayHotkeyAgain(false);

          setPendingGameplayHotkey({
            target: capturingHotkey,
            hotkey: capturedHotkey,
          });

          setCapturingHotkey(null);
          setHotkeyMessage(null);

          return;
        }
      }

      void saveCapturedHotkey(
        capturingHotkey,
        capturedHotkey,
      );
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [capturingHotkey, hotkeySettingsSaving, saveCapturedHotkey]);


  const cancelGameplayHotkey = useCallback(() => {
    setPendingGameplayHotkey(null);
    setDontRemindGameplayHotkeyAgain(false);
  }, []);

  const confirmGameplayHotkey = useCallback(async () => {
    const pending = pendingGameplayHotkey;

    if (!pending) {
      return;
    }

    if (dontRemindGameplayHotkeyAgain) {
      localStorage.setItem(
        GAMEPLAY_HOTKEY_WARNING_KEY,
        "true",
      );
    }

    setPendingGameplayHotkey(null);
    setDontRemindGameplayHotkeyAgain(false);

    await saveCapturedHotkey(
      pending.target,
      pending.hotkey,
    );
  }, [
    pendingGameplayHotkey,
    dontRemindGameplayHotkeyAgain,
    saveCapturedHotkey,
  ]);


  const resetHotkeys = useCallback(async () => {
    setHotkeySettingsSaving(true);
    setHotkeyMessage(null);
    setCapturingHotkey(null);
    try {
      const saved = await invoke<HotkeySettings>("reset_hotkey_settings");
      setHotkeySettings(saved);
      setHotkeysRestored(true);
      window.setTimeout(() => setHotkeysRestored(false), 1500);
    } catch (reason) {
      setHotkeyMessage(String(reason));
    } finally {
      setHotkeySettingsSaving(false);
    }
  }, []);

  const restoreClearPresetConfirmation =
    useCallback(() => {
      localStorage.removeItem(
        CLEAR_PRESET_CONFIRMATION_KEY,
      );
      setClearPresetConfirmationRestored(true);

      window.setTimeout(() => {
        setClearPresetConfirmationRestored(false);
      }, 1500);
    }, []);


  const restoreGameplayHotkeyWarning =
    useCallback(() => {
      localStorage.removeItem(
        GAMEPLAY_HOTKEY_WARNING_KEY,
      );

      setGameplayHotkeyWarningRestored(true);

      window.setTimeout(() => {
        setGameplayHotkeyWarningRestored(false);
      }, 1500);
    }, []);  

  const restoreFavoriteModeWarning =
    useCallback(() => {
      localStorage.removeItem(
        FAVORITE_MODE_WARNING_KEY,
      );

      setFavoriteModeWarningRestored(true);

      window.setTimeout(() => {
        setFavoriteModeWarningRestored(false);
      }, 1500);
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
    <div className="shell">
      
      {pendingGameplayHotkey && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="gameplay-hotkey-warning-title"
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
                  id="gameplay-hotkey-warning-title"
                  className="confirmation-title"
                >
                  This shortcut may interfere with gameplay
                </h3>

                <p className="confirmation-message">
                  While this shortcut is active, SPLIT captures
                  its key combination before Deadlock receives it.
                  If these keys are also used for movement or other
                  in-game actions, those actions may not work while
                  the shortcut is being triggered.
                </p>

                <p className="confirmation-description">
                  For example, assigning Ctrl + Z may prevent Z
                  movement while Ctrl is held (if Ctrl is also used
                  for an in-game action, such as crouch).
                </p>

                <p className="confirmation-description">
                  Selected shortcut:{" "}
                  <strong>
                    {formatHotkey(
                      pendingGameplayHotkey.hotkey,
                    )}
                  </strong>
                </p>
              </div>
            </div>


            <label className="confirmation-checkbox">
              <input
                type="checkbox"
                checked={dontRemindGameplayHotkeyAgain}
                onChange={(event) =>
                  setDontRemindGameplayHotkeyAgain(
                    event.target.checked,
                  )
                }
              />

              <span>
                Don't remind me again
              </span>
            </label>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                onClick={cancelGameplayHotkey}
              >
                Cancel
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={() =>
                  void confirmGameplayHotkey()
                }
              >
                Use anyway
              </button>
            </div>
          </section>
        </div>
      )}
      

      {favoriteModeWarningOpen && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="favorite-mode-warning-title"
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
                  id="favorite-mode-warning-title"
                  className="confirmation-title"
                >
                  Favorite Mode has no presets
                </h3>

                <p className="confirmation-message">
                  Favorite Mode uses a single set of
                  8 Favorites.
                </p>

                <p className="confirmation-description">
                  Presets do not apply while Favorite
                  Mode is active.
                </p>

                <p className="confirmation-description">
                  Any changes you make here affect
                  these same 8 Favorites.
                </p>
              </div>
            </div>

            <label className="confirmation-checkbox">
              <input
                type="checkbox"
                checked={dontRemindFavoriteModeAgain}
                onChange={(event) =>
                  setDontRemindFavoriteModeAgain(
                    event.target.checked,
                  )
                }
              />

              <span>
                Don't remind me again
              </span>
            </label>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                onClick={() =>
                  void leaveFavoriteMode()
                }
              >
                Leave Favorites
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={continueFavoriteMode}
              >
                Continue
              </button>
            </div>
          </section>
        </div>
      )}

      {pendingFavoriteCopy && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="save-favorite-title"
          >
            <div className="confirmation-content">
              <span
                className="confirmation-warning"
                aria-hidden="true"
              >
                ★
              </span>

              <div>
                <h3
                  id="save-favorite-title"
                  className="confirmation-title"
                >
                  Save to Favorite
                </h3>

                <p className="confirmation-message">
                  Save{" "}
                  <strong>
                    &quot;
                    {pendingFavoriteCopy.sourceName}
                    &quot;
                  </strong>{" "}
                  to which Favorite slot?
                </p>

                <p className="confirmation-description">
                  The complete save will be copied,
                  including position, camera, name,
                  timestamp and color.
                </p>
              </div>
            </div>

            <div className="preset-switcher">
              {favoriteSlotSummaries.map(
                (favorite) => (
                  <button
                    key={favorite.slot}
                    className="preset-button"
                    type="button"
                    disabled={favoriteCopyWorking}
                    onClick={() =>
                      void chooseFavoriteDestination(
                        favorite,
                      )
                    }
                  >
                    Favorite {favorite.slot}
                    {" · "}
                    {favorite.occupied
                      ? favorite.name
                      : "Empty"}
                  </button>
                ),
              )}
            </div>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                disabled={favoriteCopyWorking}
                onClick={cancelFavoriteCopy}
              >
                Cancel
              </button>
            </div>
          </section>
        </div>
      )}

      {pendingFavoriteOverwrite && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="overwrite-favorite-title"
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
                  id="overwrite-favorite-title"
                  className="confirmation-title"
                >
                  Replace Favorite?
                </h3>

                <p className="confirmation-message">
                  Favorite{" "}
                  {pendingFavoriteOverwrite.target.slot}
                  {" "}
                  currently contains{" "}
                  <strong>
                    &quot;
                    {pendingFavoriteOverwrite.target.name}
                    &quot;
                  </strong>
                  .
                </p>

                <p className="confirmation-description">
                  Replace it with{" "}
                  <strong>
                    &quot;
                    {pendingFavoriteOverwrite.sourceName}
                    &quot;
                  </strong>
                  ? This action cannot be undone.
                </p>
              </div>
            </div>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                disabled={favoriteCopyWorking}
                onClick={cancelFavoriteOverwrite}
              >
                Back
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                disabled={favoriteCopyWorking}
                onClick={() =>
                  void confirmFavoriteOverwrite()
                }
              >
                {favoriteCopyWorking
                  ? "Replacing…"
                  : "Replace Favorite"}
              </button>
            </div>
          </section>
        </div>
      )}


      {pendingImportPreset && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="import-preset-title"
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
                  id="import-preset-title"
                  className="confirmation-title"
                >
                  Import preset
                </h3>

                <p className="confirmation-message">
                  Import{" "}
                  <strong>
                    &quot;
                    {pendingImportPreset.importedName}
                    &quot;
                  </strong>{" "}
                  into{" "}
                  <strong>
                    &quot;
                    {pendingImportPreset.currentName}
                    &quot;
                  </strong>
                  ?
                </p>

                <p className="confirmation-description">
                  This will replace all 8 slots
                  and the current preset name.
                </p>
              </div>
            </div>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                onClick={cancelPresetImport}
              >
                Cancel
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={() =>
                  void confirmPresetImport()
                }
              >
                Import preset
              </button>
            </div>
          </section>
        </div>
      )}

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

      <aside className="sidebar">
        <div className="brand" aria-label="SPLIT version 2">
          <strong>SPLIT</strong>
          <span>v2</span>
        </div>

        <nav className="sidebar-nav" aria-label="Main navigation">
          <button
            type="button"
            className={activeView === "slots" && !favoriteMode ? "active" : ""}
            onClick={() => {
              setActiveView("slots");
              if (favoriteMode) void leaveFavoriteMode();
            }}
          >
            <span>Presets</span>
          </button>
          <button
            type="button"
            className={activeView === "slots" && favoriteMode ? "active" : ""}
            disabled={savingSlot !== null || loadingSlot !== null}
            onClick={() => {
              setActiveView("slots");
              if (!favoriteMode) void toggleFavorites();
            }}
          >
            <span>Favorites</span>
            <kbd>F11</kbd>
          </button>
          <button
            type="button"
            className={activeView === "settings" ? "active" : ""}
            onClick={() => setActiveView("settings")}
          >
            <span>Settings</span>
          </button>
        </nav>

        <div className="sidebar-status">
          <div>
            <StatusDot
              tone={status.deadlockRunning ? "ok" : "off"}
            />
            <strong>Deadlock</strong>
          </div>
          <span>{status.deadlockRunning ? "Connected" : "Not connected"}</span>
        </div>
      </aside>

      <main className="workspace">
      <header className={`topbar ${activeView === "slots" ? "slots-topbar" : ""}`}>
        <div>
          <h1>
            {activeView === "settings"
              ? "Settings"
              : favoriteMode
                ? "Favorites"
                : "Position slots"}
          </h1>

          {(activeView === "settings" || !favoriteMode) && (
            <p className="subtitle">
              {activeView === "settings"
                ? "Preferences, shortcuts and diagnostics"
                : presetNames[activePreset - 1] ?? `Preset ${activePreset}`}
            </p>
          )}
        </div>

        {activeView === "slots" ? (
          <div className="history-actions header-history-actions">
            <button
              className="preset-button"
              type="button"
              disabled={
                !historyState.canUndo ||
                savingSlot !== null ||
                loadingSlot !== null
              }
              onClick={() => void runHistoryAction("undo_last_action")}
            >
              Undo <kbd>F9</kbd>
            </button>
            <button
              className="preset-button"
              type="button"
              disabled={
                !historyState.canRedo ||
                savingSlot !== null ||
                loadingSlot !== null
              }
              onClick={() => void runHistoryAction("redo_last_action")}
            >
              Redo <kbd>F10</kbd>
            </button>
          </div>
        ) : (
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
        )}
      </header>

      {activeView === "settings" && (
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
      )}

      {activeView === "slots" && (
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


      {!favoriteMode && <div className="preset-toolbar">
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

      <details className="preset-actions-menu">
        <summary aria-label="Preset actions" title="Preset actions">...</summary>
        <div
          className="preset-management"
          onClick={(event) => {
            if (event.target instanceof HTMLButtonElement) {
              event.currentTarget.closest("details")?.removeAttribute("open");
            }
          }}
        >
        <button
          className="preset-button"
          type="button"
          disabled={
            favoriteMode ||
            renamingPreset ||
            clearingPreset ||
            exportingPreset ||
            importingPreset ||
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
          className="preset-button"
          type="button"
          disabled={
            favoriteMode ||
            renamingPreset ||
            clearingPreset ||
            exportingPreset ||
            importingPreset ||
            savingSlot !== null ||
            loadingSlot !== null ||
            coloringSlot !== null
          }
          onClick={() =>
            void exportActivePreset()
          }
        >
          {exportingPreset
            ? "Exporting…"
            : "Export preset"}
        </button>

        <button
          className="preset-button"
          type="button"
          disabled={
            favoriteMode ||
            renamingPreset ||
            clearingPreset ||
            exportingPreset ||
            importingPreset ||
            savingSlot !== null ||
            loadingSlot !== null ||
            coloringSlot !== null
          }
          onClick={() =>
            void selectPresetImport()
          }
        >
          {importingPreset
            ? "Importing…"
            : "Import preset"}
        </button>

        <button
          className="preset-button preset-clear-button"
          type="button"
          disabled={
            favoriteMode ||
            renamingPreset ||
            clearingPreset ||
            exportingPreset ||
            importingPreset ||
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
      </details>
      </div>}

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
                className={`slot-card ${
                  position ? "filled" : "empty"
                }`}
                key={slot}
                onMouseEnter={(event) => {
                  document
                    .querySelectorAll<HTMLDetailsElement>(
                      ".slot-card-menu[open]",
                    )
                    .forEach((details) => {
                      const parentCard =
                        details.closest(".slot-card");

                      if (
                        parentCard !==
                        event.currentTarget
                      ) {
                        details.removeAttribute(
                          "open",
                        );
                      }
                    });
                }}
              >
                {metadata?.color && (
                  <span
                    className="slot-card-accent"
                    style={{
                      backgroundColor:
                        metadata.color,
                    }}
                  />
                )}

                <div
                  className={`slot-preview ${
                    position ? "filled" : "empty"
                  }`}
                >
                  <span className="slot-preview-number">
                    {String(slot).padStart(2, "0")}
                  </span>

                  {position ? (
                    <>
                      <div className="slot-preview-placeholder">
                        <span>
                          Position captured
                        </span>

                        <code>
                          {position.x.toFixed(0)}
                          {"  /  "}
                          {position.y.toFixed(0)}
                          {"  /  "}
                          {position.z.toFixed(0)}
                        </code>
                      </div>

                      <button
                        className="slot-preview-load"
                        type="button"
                        disabled={
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
                    </>
                  ) : (
                    <button
                      className="slot-empty-save"
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
                      <span className="slot-empty-plus">
                        +
                      </span>

                      <span>
                        Empty slot
                      </span>

                      <small>
                        Save current position
                      </small>
                    </button>
                  )}
                </div>

                <details className="slot-card-menu">
                  <summary
                    aria-label={`Open actions for ${displayName}`}
                    title="Slot actions"
                  >
                    ···
                  </summary>

                  <div className="slot-card-menu-panel">
                    {position && (
                      <>
                        <div className="slot-menu-position">
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

                        <div className="slot-menu-divider" />

                        <button
                          className="slot-menu-item"
                          type="button"
                          disabled={
                            loadingSlot !== null ||
                            savingSlot !== null
                          }
                          onClick={(event) => {
                            event.currentTarget
                              .closest("details")
                              ?.removeAttribute("open");

                            void loadSavedSlot(
                              slot,
                            );
                          }}
                        >
                          Load
                          <span>
                            {formatHotkey(
                              hotkeySettings.loadSlots[
                                slot - 1
                              ],
                            )}
                          </span>
                        </button>
                      </>
                    )}

                    <button
                      className="slot-menu-item"
                      type="button"
                      disabled={
                        savingSlot !== null ||
                        loadingSlot !== null
                      }
                      onClick={(event) => {
                        event.currentTarget
                          .closest("details")
                          ?.removeAttribute("open");

                        void saveCurrentToSlot(
                          slot,
                        );
                      }}
                    >
                      {position
                        ? "Overwrite"
                        : "Save"}
                      <span>
                        {formatHotkey(
                          hotkeySettings.saveSlots[
                            slot - 1
                          ],
                        )}
                      </span>
                    </button>

                    <button
                      className="slot-menu-item"
                      type="button"
                      disabled={
                        savingSlot !== null ||
                        loadingSlot !== null
                      }
                      onClick={(event) => {
                        event.currentTarget
                          .closest("details")
                          ?.removeAttribute("open");

                        void renameSavedSlot(
                          slot,
                          displayName,
                        );
                      }}
                    >
                      Rename
                    </button>

                    {!favoriteMode && position && (
                      <button
                        className="slot-menu-item"
                        type="button"
                        disabled={
                          favoriteCopyWorking ||
                          savingSlot !== null ||
                          loadingSlot !== null ||
                          coloringSlot !== null
                        }
                        onClick={(event) => {
                          event.currentTarget
                            .closest("details")
                            ?.removeAttribute("open");

                          void openSaveToFavorite(
                            slot,
                            displayName,
                          );
                        }}
                      >
                        Save to Favorite
                      </button>
                    )}

                    {position && (
                      <>
                        <div className="slot-menu-divider" />

                        <span className="slot-menu-label">
                          Color
                        </span>

                        <div className="slot-menu-colors">
                          {SLOT_COLORS.map(
                            ({ label, value }) => {
                              const active =
                                metadata?.color ===
                                value;

                              return (
                                <button
                                  className={`slot-menu-color ${
                                    active
                                      ? "active"
                                      : ""
                                  }`}
                                  type="button"
                                  key={label}
                                  title={label}
                                  aria-label={`${label} slot color`}
                                  disabled={
                                    savingSlot !==
                                      null ||
                                    loadingSlot !==
                                      null ||
                                    coloringSlot !==
                                      null
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
                                      className="slot-menu-color-swatch"
                                      style={{
                                        backgroundColor:
                                          value,
                                      }}
                                    />
                                  ) : (
                                    <span className="slot-menu-color-none">
                                      ×
                                    </span>
                                  )}
                                </button>
                              );
                            },
                          )}
                        </div>
                      </>
                    )}

                    <div className="slot-menu-divider" />

                    <button
                      className="slot-menu-item danger"
                      type="button"
                      disabled={
                        !canClear ||
                        savingSlot !== null ||
                        loadingSlot !== null
                      }
                      onClick={(event) => {
                        event.currentTarget
                          .closest("details")
                          ?.removeAttribute("open");

                        void clearSavedSlot(
                          slot,
                          displayName,
                        );
                      }}
                    >
                      Clear
                    </button>
                  </div>
                </details>

                <div className="slot-card-info">
                  <div className="slot-card-copy">
                    <strong title={displayName}>
                      {displayName}
                    </strong>

                    <span>
                      {savedAge ??
                        (position
                          ? "Saved"
                          : "No position saved")}
                    </span>
                  </div>

                  <div className="slot-card-hotkey">
                    {position
                      ? formatHotkey(
                          hotkeySettings.loadSlots[
                            slot - 1
                          ],
                        )
                      : formatHotkey(
                          hotkeySettings.saveSlots[
                            slot - 1
                          ],
                        )}
                  </div>
                </div>
              </article>
            );
          },
        )}
      </div>
    </section>
      )}

      {activeView === "settings" && (
      <div className="settings-view">
      <section className="hotkey-settings-section">
        <div className="hotkey-settings-heading">
          <div>
            <p className="label">HOTKEYS</p>
            <h2>Keyboard shortcuts</h2>
          </div>
          <button
            className="preset-button"
            type="button"
            disabled={hotkeySettingsSaving}
            onClick={() => void resetHotkeys()}
          >
            {hotkeysRestored ? "Restored" : "Reset to defaults"}
          </button>
        </div>

        <p className="hotkey-settings-note">
          Assigned hotkeys are captured by SPLIT while Deadlock is focused.
        </p>

        <div className="hotkey-categories">
          {([
            ["LOAD", "loadSlots", hotkeySettings.loadSlots],
            ["SAVE", "saveSlots", hotkeySettings.saveSlots],
          ] as const).map(([label, group, bindings]) => (
            <div className="hotkey-category" key={group}>
              <h3>{label}</h3>
              {bindings.map((hotkey, index) => {
                const target: HotkeyTarget = { group, index };
                const capturing = isHotkeyTarget(capturingHotkey, target);
                return (
                  <div className={`hotkey-row ${capturing ? "capturing" : ""}`} key={`${group}-${index}`}>
                    <span>{label === "LOAD" ? "Load" : "Save"} Slot {index + 1}</span>
                    <kbd>{capturing ? "Press a shortcut…" : formatHotkey(hotkey)}</kbd>
                    <button
                      type="button"
                      disabled={hotkeySettingsSaving}
                      onClick={() => {
                        setHotkeyMessage(null);
                        setCapturingHotkey(capturing ? null : target);
                      }}
                    >
                      {capturing ? "Cancel" : "Change"}
                    </button>
                  </div>
                );
              })}
            </div>
          ))}

          <div className="hotkey-category hotkey-actions-category">
            <h3>ACTIONS</h3>
            {([
              ["Undo", "undo", hotkeySettings.undo],
              ["Redo", "redo", hotkeySettings.redo],
              ["Cycle Preset", "cyclePreset", hotkeySettings.cyclePreset],
              ["Favorite Mode", "favoriteMode", hotkeySettings.favoriteMode],
            ] as const).map(([label, group, hotkey]) => {
              const target: HotkeyTarget = { group };
              const capturing = isHotkeyTarget(capturingHotkey, target);
              return (
                <div className={`hotkey-row ${capturing ? "capturing" : ""}`} key={group}>
                  <span>{label}</span>
                  <kbd>{capturing ? "Press a shortcut…" : formatHotkey(hotkey)}</kbd>
                  <button
                    type="button"
                    disabled={hotkeySettingsSaving}
                    onClick={() => {
                      setHotkeyMessage(null);
                      setCapturingHotkey(capturing ? null : target);
                    }}
                  >
                    {capturing ? "Cancel" : "Change"}
                  </button>
                </div>
              );
            })}
          </div>
        </div>

        {hotkeyMessage && <p className="hotkey-message" role="alert">{hotkeyMessage}</p>}
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

          <div className="notification-setting-row">
            <span>Clear preset confirmation</span>
            <button
              className="notification-toggle"
              type="button"
              onClick={restoreClearPresetConfirmation}
            >
              {clearPresetConfirmationRestored
                ? "Restored"
                : "Restore"}
            </button>
          </div>

          <div className="notification-setting-row">
            <span>Gameplay hotkey warning</span>

            <button
              className="notification-toggle"
              type="button"
              onClick={restoreGameplayHotkeyWarning}
            >
              {gameplayHotkeyWarningRestored
                ? "Restored"
                : "Restore"}
            </button>
          </div>

          <div className="notification-setting-row">
            <span>Favorite Mode warning</span>

            <button
              className="notification-toggle"
              type="button"
              onClick={restoreFavoriteModeWarning}
            >
              {favoriteModeWarningRestored
                ? "Restored"
                : "Restore"}
            </button>
          </div>      


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
      </div>
      )}

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
    </div>
  );
}

export default App;
