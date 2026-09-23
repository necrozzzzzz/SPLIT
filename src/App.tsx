import {
  useCallback,
  useEffect,
  useState,
} from "react";

import {
  convertFileSrc,
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
} from "@tauri-apps/plugin-fs";

import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";

import {
  getFullScreenshotPath,
} from "./screenshot";

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
  screenshot: string | null;
};

type FavoriteSlotSummary = {
  slot: number;
  occupied: boolean;
  name: string;
  savedAt: number | null;
  color: string | null;
};

type SlotMetadataExport = {
  name: string;
  savedAt: number | null;
  color: string | null;
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
  quickAccess: Hotkey;
};

type QuickAccessSettings = {
  enabled: boolean;
  position: "left" | "right";
};

type HotkeyTarget =
  | { group: "loadSlots" | "saveSlots"; index: number }
  | { group: "undo" | "redo" | "cyclePreset" | "favoriteMode" | "quickAccess" };

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

const SLOT_COLOR_DISPLAY_MODE_KEY =
  "split.slotColorDisplayMode";


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
  quickAccess: {
    key: "CapsLock",
    ctrl: false,
    alt: false,
    shift: false,
  },
};

const DEFAULT_QUICK_ACCESS_SETTINGS: QuickAccessSettings = {
  enabled: true,
  position: "left",
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
    Spacebar: "Space", CapsLock: "CapsLock",
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

  if (
    target.group !== "quickAccess" &&
    hotkeysEqual(
      settings.quickAccess,
      candidate,
    )
  ) {
    return "Quick Access";
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

type SlotColorDisplayMode =
  | "tint"
  | "accent"
  | "dot";

type SettingsSection =
  | "general"
  | "hotkeys"
  | "notifications"
  | "diagnostics";

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
    activeSettingsSection,
    setActiveSettingsSection,
  ] = useState<SettingsSection>(
    "general",
  );  


  const [
    autostartEnabled,
    setAutostartEnabled,
  ] = useState(false);

  const [
    autostartLoading,
    setAutostartLoading,
  ] = useState(true);

  const [
    autostartSaving,
    setAutostartSaving,
  ] = useState(false);

  const [
    startMinimized,
    setStartMinimized,
  ] = useState(false);

  const [
    startMinimizedLoading,
    setStartMinimizedLoading,
  ] = useState(true);

  const [
    startMinimizedSaving,
    setStartMinimizedSaving,
  ] = useState(false);

  const [
    slotColorDisplayMode,
    setSlotColorDisplayMode,
  ] = useState<SlotColorDisplayMode>(() => {
    const stored =
      localStorage.getItem(
        SLOT_COLOR_DISPLAY_MODE_KEY,
      );

    if (
      stored === "tint" ||
      stored === "accent" ||
      stored === "dot"
    ) {
      return stored;
    }

    return "tint";
  });

  const updateSlotColorDisplayMode = (
    mode: SlotColorDisplayMode,
  ) => {
    setSlotColorDisplayMode(mode);

    localStorage.setItem(
      SLOT_COLOR_DISPLAY_MODE_KEY,
      mode,
    );
  };

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
          screenshot: null,
        }),
      ),
  );

  const [
    screenshotViewer,
    setScreenshotViewer,
  ] = useState<{
    name: string;
    thumbnailPath: string;
  } | null>(null);


  useEffect(() => {
    if (!screenshotViewer) {
      return;
    }

    const onKeyDown = (
      event: KeyboardEvent,
    ) => {
      if (event.key === "Escape") {
        setScreenshotViewer(null);
      }
    };

    window.addEventListener(
      "keydown",
      onKeyDown,
    );

    return () => {
      window.removeEventListener(
        "keydown",
        onKeyDown,
      );
    };
  }, [screenshotViewer]);

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
    source:
      | {
          kind: "archive";
          path: string;
        }
      | {
          kind: "legacy";
          imported: unknown;
        };
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

  const [quickAccessSettings, setQuickAccessSettings] =
    useState<QuickAccessSettings>(DEFAULT_QUICK_ACCESS_SETTINGS);
  const [quickAccessSettingsLoading, setQuickAccessSettingsLoading] =
    useState(true);
  const [quickAccessSettingsSaving, setQuickAccessSettingsSaving] =
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
    launchingDeadlock,
    setLaunchingDeadlock,
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
    let disposed = false;

    void isAutostartEnabled()
      .then((enabled) => {
        if (!disposed) {
          setAutostartEnabled(enabled);
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setError(
            `Could not read startup setting: ${String(reason)}`,
          );
        }
      })
      .finally(() => {
        if (!disposed) {
          setAutostartLoading(false);
        }
      });

    return () => {
      disposed = true;
    };
  }, []);  

  useEffect(() => {
    let disposed = false;

    void invoke<boolean>(
      "get_start_minimized_to_tray",
    )
      .then((enabled) => {
        if (!disposed) {
          setStartMinimized(
            enabled,
          );
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setError(
            `Could not read minimized startup setting: ${String(reason)}`,
          );
        }
      })
      .finally(() => {
        if (!disposed) {
          setStartMinimizedLoading(
            false,
          );
        }
      });

    return () => {
      disposed = true;
    };
  }, []);

  const toggleAutostart =
    useCallback(async () => {
      const next =
        !autostartEnabled;

      setAutostartSaving(true);
      setError(null);

      try {
        if (next) {
          await enableAutostart();
        } else {
          await disableAutostart();

          if (startMinimized) {
            await invoke<boolean>(
              "set_start_minimized_to_tray",
              {
                enabled: false,
              },
            );

            setStartMinimized(false);
          }
        }

        const enabled =
          await isAutostartEnabled();

        setAutostartEnabled(
          enabled,
        );
      } catch (reason) {
        setError(
          `Could not update startup setting: ${String(reason)}`,
        );
      } finally {
        setAutostartSaving(false);
      }
    }, [
      autostartEnabled,
      startMinimized,
    ]);


  const toggleStartMinimized =
    useCallback(async () => {
      const next =
        !startMinimized;

      setStartMinimizedSaving(true);
      setError(null);

      try {
        const enabled =
          await invoke<boolean>(
            "set_start_minimized_to_tray",
            {
              enabled: next,
            },
          );

        setStartMinimized(
          enabled,
        );
      } catch (reason) {
        setError(
          `Could not update minimized startup setting: ${String(reason)}`,
        );
      } finally {
        setStartMinimizedSaving(
          false,
        );
      }
    }, [startMinimized]);  

  const [
    closeToTray,
    setCloseToTray,
  ] = useState(true);

  const [
    closeBehaviorLoading,
    setCloseBehaviorLoading,
  ] = useState(true);

  const [
    closeBehaviorSaving,
    setCloseBehaviorSaving,
  ] = useState(false);  

  const [
    resettingWindow,
    setResettingWindow,
  ] = useState(false);


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


  useEffect(() => {
    let disposed = false;

    void invoke<boolean>(
      "get_close_to_tray",
    )
      .then((enabled) => {
        if (!disposed) {
          setCloseToTray(enabled);
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setError(
            `Could not read close behavior: ${String(reason)}`,
          );
        }
      })
      .finally(() => {
        if (!disposed) {
          setCloseBehaviorLoading(false);
        }
      });

    return () => {
      disposed = true;
    };
  }, []);

  const updateCloseBehavior =
    useCallback(
      async (
        enabled: boolean,
      ) => {
        if (
          closeBehaviorSaving ||
          enabled === closeToTray
        ) {
          return;
        }

        setCloseBehaviorSaving(true);
        setError(null);

        try {
          const result =
            await invoke<boolean>(
              "set_close_to_tray",
              {
                enabled,
              },
            );

          setCloseToTray(result);
        } catch (reason) {
          setError(
            `Could not update close behavior: ${String(reason)}`,
          );
        } finally {
          setCloseBehaviorSaving(false);
        }
      },
      [
        closeBehaviorSaving,
        closeToTray,
      ],
    );


  const resetWindow =
    useCallback(async () => {
      setResettingWindow(true);
      setError(null);

      try {
        await invoke(
          "reset_main_window",
        );
      } catch (reason) {
        setError(
          `Could not reset window: ${String(reason)}`,
        );
      } finally {
        setResettingWindow(false);
      }
    }, []);  



  const launchDeadlock =
    useCallback(async () => {
      if (
        launchingDeadlock ||
        status.deadlockRunning
      ) {
        return;
      }

      setLaunchingDeadlock(true);
      setError(null);

      try {
        await invoke(
          "launch_deadlock",
        );

        /*
        * Attend que Deadlock soit réellement
        * détecté avant de retirer le bandeau.
        */
        for (
          let attempt = 0;
          attempt < 30;
          attempt += 1
        ) {
          await new Promise<void>(
            (resolve) => {
              window.setTimeout(
                resolve,
                1000,
              );
            },
          );

          const next =
            await invoke<DeadlockStatus>(
              "get_deadlock_status",
            );

          setStatus(next);

          if (next.deadlockRunning) {
            return;
          }
        }
      } catch (reason) {
        setError(
          `Could not launch Deadlock: ${String(reason)}`,
        );
      } finally {
        setLaunchingDeadlock(false);
      }
    }, [
      launchingDeadlock,
      status.deadlockRunning,
    ]);  


  const refresh =
    useCallback(async () => {
      setLoading(true);
      setError(null);

      try {
        const [
          nextStatus,
          position,
          nextSlots,
          nextMetadata,
          nextPreset,
          nextPresetNames,
          nextHistory,
          nextFavoriteMode,
        ] = await Promise.all([
          invoke<DeadlockStatus>(
            "get_deadlock_status",
          ),
          invoke<PositionSnapshot | null>(
            "get_last_position",
          ),
          invoke<Array<PositionSnapshot | null>>(
            "get_slots",
          ),
          invoke<Array<SlotMetadata>>(
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

        setStatus(nextStatus);

        if (position) {
          setLastPosition(
            position,
          );
        }
        setSlots(nextSlots);
        setSlotMetadata(nextMetadata);
        setActivePreset(nextPreset);
        setPresetNames(nextPresetNames);
        setHistoryState(nextHistory);
        setFavoriteMode(nextFavoriteMode);
        setRelativeTimeNow(Date.now());
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

              const history =
                await invoke<HistoryState>(
                  "get_history_state",
                );

              setPresetNames(
                names,
              );

              setHistoryState(
                history,
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
              const presetName =
                presetNames[
                  activePreset - 1
                ] ??
                `Preset ${activePreset}`;

              const sanitizedName =
                presetName
                  .replace(
                    /[<>:"/\\|?*\u0000-\u001f]/g,
                    "_",
                  )
                  .replace(
                    /[. ]+$/g,
                    "",
                  );

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
                    `${safeName}.splitpreset`,
                  filters: [
                    {
                      name:
                        "SPLIT preset",
                      extensions: [
                        "splitpreset",
                      ],
                    },
                  ],
                });

              if (filePath === null) {
                return;
              }

              await invoke(
                "export_preset_archive",
                {
                  preset:
                    activePreset,
                  destination:
                    filePath,
                },
              );
            } catch (reason) {
              setError(
                String(reason),
              );
            } finally {
              setExportingPreset(
                false,
              );
            }
          },
          [
            activePreset,
            presetNames,
          ],
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
                        "splitpreset",
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

              const currentName =
                presetNames[
                  activePreset - 1
                ] ??
                `Preset ${activePreset}`;

              /*
              * Nouveau format portable :
              * on ne tente surtout PAS de le lire
              * comme du texte / JSON côté frontend.
              *
              * Le backend Rust ouvrira lui-même
              * l'archive TAR.
              */
              if (
                filePath
                  .toLowerCase()
                  .endsWith(
                    ".splitpreset",
                  )
              ) {
                const fileName =
                  filePath
                    .split(/[\\/]/)
                    .pop() ??
                  "Imported preset";

                const importedName =
                  fileName.replace(
                    /\.splitpreset$/i,
                    "",
                  ) ||
                  "Imported preset";

                setPendingImportPreset({
                  preset:
                    activePreset,
                  currentName,
                  importedName,
                  source: {
                    kind: "archive",
                    path: filePath,
                  },
                });

                return;
              }

              /*
              * Compatibilité avec les anciens
              * exports JSON.
              */
              const imported: unknown =
                JSON.parse(
                  await readTextFile(
                    filePath,
                  ),
                );

              const importedName =
                typeof imported ===
                  "object" &&
                imported !== null &&
                "name" in imported &&
                typeof imported.name ===
                  "string"
                  ? imported.name
                  : "Invalid preset";

              setPendingImportPreset({
                preset:
                  activePreset,
                currentName,
                importedName,
                source: {
                  kind: "legacy",
                  imported,
                },
              });
            } catch (reason) {
              setError(
                String(reason),
              );
            } finally {
              setImportingPreset(
                false,
              );
            }
          },
          [
            activePreset,
            presetNames,
          ],
        );

      const confirmPresetImport =
        useCallback(
          async () => {
            const target =
              pendingImportPreset;

            if (!target) {
              return;
            }

            setPendingImportPreset(
              null,
            );

            setImportingPreset(
              true,
            );

            setError(null);

            try {
              let result:
                SlotEditResult;

              if (
                target.source.kind ===
                "archive"
              ) {
                result =
                  await invoke<SlotEditResult>(
                    "import_preset_archive",
                    {
                      preset:
                        target.preset,
                      source:
                        target.source.path,
                    },
                  );
              } else {
                result =
                  await invoke<SlotEditResult>(
                    "import_preset",
                    {
                      preset:
                        target.preset,
                      imported:
                        target.source
                          .imported,
                    },
                  );
              }

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
              setImportingPreset(
                false,
              );
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

          const names =
            await invoke<
              Array<string>
            >(
              "get_preset_names",
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

          setPresetNames(
            names,
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

          const history =
            await invoke<HistoryState>(
              "get_history_state",
            );

          setHistoryState(
            history,
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

  useEffect(() => {
    let disposed = false;
    invoke<QuickAccessSettings>(
      "get_quick_access_settings",
    )
      .then((saved) => {
        if (!disposed) {
          setQuickAccessSettings(saved);
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setError(String(reason));
        }
      })
      .finally(() => {
        if (!disposed) {
          setQuickAccessSettingsLoading(false);
        }
      });

    return () => {
      disposed = true;
    };
  }, []);

  const updateQuickAccessSettings = useCallback(
    async (next: QuickAccessSettings) => {
      const previous = quickAccessSettings;
      if (
        !next.enabled &&
        capturingHotkey?.group === "quickAccess"
      ) {
        setCapturingHotkey(null);
        setHotkeyMessage(null);
      }
      setQuickAccessSettings(next);
      setQuickAccessSettingsSaving(true);
      setError(null);

      try {
        const saved = await invoke<QuickAccessSettings>(
          "update_quick_access_settings",
          { settings: next },
        );
        setQuickAccessSettings(saved);
      } catch (reason) {
        setQuickAccessSettings(previous);
        setError(String(reason));
      } finally {
        setQuickAccessSettingsSaving(false);
      }
    },
    [capturingHotkey, quickAccessSettings],
  );

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
        <section className="setup-card setup-loading-card">
          <div className="setup-loading-content">
            <p className="eyebrow">
              SPLIT 2
            </p>

            <div className="setup-loading-heading">
              <div className="setup-loading-icon">
                <span />
              </div>

              <div>
                <h1>
                  Detecting Deadlock
                </h1>

                <p className="setup-description">
                  Scanning your Steam libraries for a
                  Deadlock installation.
                </p>
              </div>
            </div>

            <div className="setup-scan-status">
              <div className="setup-scan-status-header">
                <span>INSTALLATION SCAN</span>
                <strong>Searching…</strong>
              </div>

              <div className="setup-scan-track">
                <span />
              </div>

              <p>
                Checking Steam libraries and known
                installation locations.
              </p>
            </div>
          </div>
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
        <section className="setup-card setup-onboarding-card">
          <div className="setup-onboarding-grid">
            <div className="setup-onboarding-main">
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
                      disabled={setupWorking}
                      onClick={() =>
                        void confirmPath(
                          detected,
                        )
                      }
                    >
                      {setupWorking
                        ? "Configuring…"
                        : "Yes, continue"}
                    </button>

                    <button
                      className="secondary-button"
                      type="button"
                      disabled={setupWorking}
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
                    SPLIT couldn't detect a Deadlock installation
                    in your Steam libraries.
                  </p>

                  <div className="setup-manual-folder">
                    <div className="setup-manual-folder-icon">
                      <svg
                        viewBox="0 0 24 24"
                        aria-hidden="true"
                      >
                        <path
                          d="M3.75 6.75A1.75 1.75 0 0 1 5.5 5h4.1c.46 0 .9.18 1.22.5l1.18 1.18c.19.2.46.32.74.32h5.76a1.75 1.75 0 0 1 1.75 1.75v7.75a1.75 1.75 0 0 1-1.75 1.75h-13A1.75 1.75 0 0 1 3.75 16.5V6.75Z"
                        />
                      </svg>
                    </div>

                    <div className="setup-manual-folder-content">
                      <span>MANUAL SELECTION</span>

                      <strong>
                        Select your Deadlock folder
                      </strong>

                      <p>
                        Choose the main installation directory,
                        usually located inside:
                      </p>

                      <code>
                        steamapps\common\Deadlock
                      </code>
                    </div>
                  </div>

                  <div className="setup-actions">
                    <button
                      className="primary-button"
                      type="button"
                      disabled={setupWorking}
                      onClick={() => void chooseFolder()}
                    >
                      Choose Deadlock folder
                    </button>

                    <button
                      className="secondary-button"
                      type="button"
                      disabled={setupWorking}
                      onClick={() => void rescan()}
                    >
                      Scan again
                    </button>
                  </div>
                </>
              )}

              {setupWorking && (
                <p className="setup-working">
                  Setting up SPLIT integration…
                </p>
              )}

              {error && (
                <div className="error-box">
                  {error}
                </div>
              )}
            </div>

            <aside
              className="setup-progress-panel"
              aria-label="Setup progress"
            >
              <span className="setup-progress-label">
                SETUP
              </span>

              <div
                className={`setup-progress-step ${
                  detected
                    ? "complete"
                    : "warning"
                }`}
              >
                <span className="setup-step-index">
                  01
                </span>

                <div>
                  <strong>
                    Deadlock detection
                  </strong>

                  <small>
                    {detected
                      ? "Installation found"
                      : "Not detected automatically"}
                  </small>
                </div>
              </div>

              <div
                className={`setup-progress-step ${
                  setupWorking ? "complete" : "active"
                }`}
              >
                <span className="setup-step-index">
                  02
                </span>

                <div>
                  <strong>
                    {detected
                      ? "Confirm folder"
                      : "Choose folder"}
                  </strong>

                  <small>
                    {setupWorking
                      ? "Installation path verified"
                      : "Verify the installation path"}
                  </small>
                </div>
              </div>

              <div
                className={`setup-progress-step ${
                  setupWorking ? "active" : ""
                }`}
              >
                <span className="setup-step-index">
                  03
                </span>

                <div>
                  <strong>
                    SPLIT integration
                  </strong>

                  <small>
                    {setupWorking
                      ? "Configuring Deadlock…"
                      : "Configure Deadlock automatically"}
                  </small>
                </div>
              </div>

              <div className="setup-progress-info">
                <span aria-hidden="true">
                  i
                </span>

                <p>
                  SPLIT will configure the required
                  Deadlock integration automatically
                  after confirmation.
                </p>
              </div>
            </aside>
          </div>
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

  const screenshotViewerFullPath =
    screenshotViewer
      ? getFullScreenshotPath(
          screenshotViewer.thumbnailPath,
        )
      : null;      

  return (
    <div
      className="shell"
      onContextMenu={(event) => {
        event.preventDefault();
      }}
    >

      {screenshotViewer && (
        <div
          className="screenshot-viewer-backdrop"
          onClick={() =>
            setScreenshotViewer(null)
          }
        >
          <section
            className="screenshot-viewer"
            role="dialog"
            aria-modal="true"
            aria-label={`Screenshot for ${screenshotViewer.name}`}
            onClick={(event) =>
              event.stopPropagation()
            }
          >
            <header className="screenshot-viewer-header">
              <div>
                <strong>
                  {screenshotViewer.name}
                </strong>

                <span>
                  Saved screenshot
                </span>
              </div>

              <button
                type="button"
                aria-label="Close screenshot"
                title="Close"
                onClick={() =>
                  setScreenshotViewer(null)
                }
              >
                ×
              </button>
            </header>

            <div className="screenshot-viewer-image">
              <img
                src={convertFileSrc(
                  screenshotViewerFullPath ??
                    screenshotViewer.thumbnailPath,
                )}
                alt={`Screenshot for ${screenshotViewer.name}`}
                draggable={false}
                onError={(event) => {
                  /*
                  * Ancien Save ou full-res pas encore
                  * écrit : fallback vers le thumbnail.
                  */
                  if (
                    screenshotViewerFullPath ===
                      null ||
                    event.currentTarget.dataset
                      .fallback === "true"
                  ) {
                    return;
                  }

                  event.currentTarget.dataset.fallback =
                    "true";

                  event.currentTarget.src =
                    convertFileSrc(
                      screenshotViewer.thumbnailPath,
                    );
                }}
              />
            </div>

            <footer className="screenshot-viewer-footer">
              Right click a saved slot to open its screenshot · Esc to close
            </footer>
          </section>
        </div>
      )}
      
      {pendingGameplayHotkey && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog gameplay-hotkey-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="gameplay-hotkey-warning-title"
          >
            <div className="confirmation-content">
              <span
                className="confirmation-warning gameplay-hotkey-warning"
                aria-hidden="true"
              >
                !
              </span>

              <div className="gameplay-hotkey-content">
                <h3
                  id="gameplay-hotkey-warning-title"
                  className="confirmation-title"
                >
                  This shortcut may interfere with gameplay
                </h3>

                <p className="confirmation-description">
                  SPLIT captures this key combination before
                  Deadlock receives it.
                </p>

                <div className="gameplay-hotkey-selected">
                  <span>Selected shortcut</span>

                  <strong>
                    {formatHotkey(
                      pendingGameplayHotkey.hotkey,
                    )}
                  </strong>
                </div>

                <p className="gameplay-hotkey-note">
                  If these keys are also used in-game, those
                  actions may not work while the shortcut is
                  being triggered.
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
            className="confirmation-dialog favorite-mode-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="favorite-mode-warning-title"
          >
            <div className="confirmation-content">
              <span
                className="confirmation-warning favorite-mode-warning"
                aria-hidden="true"
              >
                ★
              </span>

              <div className="favorite-mode-content">
                <h3
                  id="favorite-mode-warning-title"
                  className="confirmation-title"
                >
                  Enter Favorite Mode?
                </h3>

                <p className="confirmation-description">
                  Favorite Mode uses one shared bank of
                  8 Favorites instead of presets.
                </p>

                <div className="favorite-mode-summary">
                  <div>
                    <span>Presets</span>
                    <strong>Disabled</strong>
                  </div>

                  <div>
                    <span>Favorites</span>
                    <strong>8 shared slots</strong>
                  </div>
                </div>

                <p className="favorite-mode-note">
                  Changes made here affect these same
                  Favorites every time you return.
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
                className="preset-button confirmation-primary-button"
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
            className="confirmation-dialog favorite-copy-dialog"
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
                  Choose a Favorite slot for{" "}
                  <strong>
                    &quot;
                    {pendingFavoriteCopy.sourceName}
                    &quot;
                  </strong>
                </h3>

                <p className="confirmation-description">
                  The complete save will be copied,
                  including position, camera, name,
                  timestamp and color.
                </p>
              </div>
            </div>

            <div className="favorite-destination-grid">
              {favoriteSlotSummaries.map(
                (favorite) => (
                  <button
                    key={favorite.slot}
                    className={`favorite-destination-card ${
                      favorite.occupied
                        ? "occupied"
                        : "empty"
                    }`}
                    type="button"
                    disabled={favoriteCopyWorking}
                    onClick={() =>
                      void chooseFavoriteDestination(
                        favorite,
                      )
                    }
                  >
                    <div className="favorite-destination-card-header">
                      <span className="favorite-destination-slot">
                        Favorite {favorite.slot}
                      </span>

                      <span
                        className={`favorite-destination-state ${
                          favorite.occupied
                            ? "occupied"
                            : "empty"
                        }`}
                      >
                        {favorite.occupied
                          ? "Occupied"
                          : "Empty"}
                      </span>
                    </div>

                    <strong className="favorite-destination-name">
                      {favorite.occupied
                        ? favorite.name
                        : "Available slot"}
                    </strong>

                    <span className="favorite-destination-hint">
                      {favorite.occupied
                        ? "Overwrite this favorite"
                        : "Save here"}
                    </span>
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
            className="confirmation-dialog favorite-overwrite-dialog"
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

              <div className="favorite-overwrite-content">
                <h3
                  id="overwrite-favorite-title"
                  className="confirmation-title"
                >
                  Replace Favorite{" "}
                  {pendingFavoriteOverwrite.target.slot}?
                </h3>

                <p className="confirmation-description">
                  This slot already contains a Favorite.
                  Choose whether to replace it with the
                  selected savestate.
                </p>

                <div className="favorite-overwrite-comparison">
                  <div>
                    <span>Current</span>

                    <strong>
                      {pendingFavoriteOverwrite.target.name}
                    </strong>
                  </div>

                  <span
                    className="favorite-overwrite-arrow"
                    aria-hidden="true"
                  >
                    →
                  </span>

                  <div>
                    <span>Replace with</span>

                    <strong>
                      {pendingFavoriteOverwrite.sourceName}
                    </strong>
                  </div>
                </div>

                <p className="favorite-overwrite-undo">
                  This change can be undone afterwards.
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
            className="confirmation-dialog preset-import-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="import-preset-title"
          >
            <div className="confirmation-content">
              <span
                className="confirmation-warning"
                aria-hidden="true"
              >
                ↓
              </span>

              <div className="preset-import-content">
                <h3
                  id="import-preset-title"
                  className="confirmation-title"
                >
                  Import preset?
                </h3>

                <p className="confirmation-description">
                  The current preset will be replaced by
                  the imported preset.
                </p>

                <div className="preset-import-comparison">
                  <div>
                    <span>Current</span>

                    <strong>
                      {pendingImportPreset.currentName}
                    </strong>
                  </div>

                  <span
                    className="preset-import-arrow"
                    aria-hidden="true"
                  >
                    →
                  </span>

                  <div>
                    <span>Import</span>

                    <strong>
                      {pendingImportPreset.importedName}
                    </strong>
                  </div>
                </div>

                <p className="preset-import-undo">
                  This change can be undone afterwards.
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
            className="confirmation-dialog clear-preset-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="clear-preset-title"
          >
            <div className="confirmation-content">
              <span
                className="confirmation-warning clear-preset-warning"
                aria-hidden="true"
              >
                !
              </span>

              <div className="clear-preset-content">
                <h3
                  id="clear-preset-title"
                  className="confirmation-title"
                >
                  Clear preset?
                </h3>

                <p className="confirmation-description">
                  All savestates in this preset will be
                  cleared.
                </p>

                <div className="clear-preset-target">
                  <span>Preset</span>

                  <strong>
                    {pendingClearPreset.name}
                  </strong>

                  <small>
                    8 savestate slots
                  </small>
                </div>

                <p className="clear-preset-undo">
                  This change can be undone afterwards.
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
            <kbd>
              {formatHotkey(
                hotkeySettings.favoriteMode,
              )}
            </kbd>
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
                : "Savestates"}
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
              Undo{" "}
              <kbd>
                {formatHotkey(
                  hotkeySettings.undo,
                )}
              </kbd>
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
              Redo{" "}
              <kbd>
                {formatHotkey(
                  hotkeySettings.redo,
                )}
              </kbd>
            </button>

            <button
              className="header-refresh-button"
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
        ) : (
          activeSettingsSection === "diagnostics" && (
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
          )
        )}
      </header>

      {activeView === "settings" && (
        <nav
          className="settings-subnav"
          aria-label="Settings sections"
        >
          <button
            type="button"
            className={
              activeSettingsSection === "general"
                ? "active"
                : ""
            }
            onClick={() =>
              setActiveSettingsSection(
                "general",
              )
            }
          >
            General
          </button>

          <button
            type="button"
            className={
              activeSettingsSection === "hotkeys"
                ? "active"
                : ""
            }
            onClick={() =>
              setActiveSettingsSection(
                "hotkeys",
              )
            }
          >
            Hotkeys
          </button>

          <button
            type="button"
            className={
              activeSettingsSection ===
              "notifications"
                ? "active"
                : ""
            }
            onClick={() =>
              setActiveSettingsSection(
                "notifications",
              )
            }
          >
            In-Game Notifications
          </button>

          <button
            type="button"
            className={
              activeSettingsSection ===
              "diagnostics"
                ? "active"
                : ""
            }
            onClick={() =>
              setActiveSettingsSection(
                "diagnostics",
              )
            }
          >
            Diagnostics
          </button>
        </nav>
      )}

      {activeView === "settings" &&
        activeSettingsSection === "diagnostics" && (
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


      {activeView === "slots" &&
        !loading &&
        !status.deadlockRunning && (
          <div className="runtime-notice">
            <div className="runtime-notice-content">
              <StatusDot tone="off" />

              <div>
                <strong>
                  Deadlock is not running
                </strong>

                <span>
                  SPLIT is ready. Hotkeys and
                  teleporting will become available
                  when Deadlock starts.
                </span>
              </div>
            </div>

            <button
              type="button"
              disabled={launchingDeadlock}
              onClick={() =>
                void launchDeadlock()
              }
            >
              {launchingDeadlock
                ? "Launching…"
                : "Launch"}
            </button>
          </div>
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


      {!favoriteMode && (
        <div className="preset-toolbar">
          <div className="preset-switcher">
            {[1, 2, 3, 4].map(
              (preset) => {
                const isActive =
                  activePreset === preset;

                return (
                  <div
                    key={preset}
                    className={`preset-tab ${
                      isActive ? "active" : ""
                    }`}
                  >
                    <button
                      type="button"
                      className={`preset-button ${
                        isActive
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

                    {isActive && (
                      <details
                        className="preset-actions-menu"
                        onMouseLeave={(event) => {
                          const details =
                            event.currentTarget;

                          window.setTimeout(() => {
                            if (!details.matches(":hover")) {
                              details.removeAttribute(
                                "open",
                              );
                            }
                          }, 90);
                        }}
                      >
                        <summary
                          aria-label="Preset actions"
                          title="Preset actions"
                        >
                          ...
                        </summary>

                        <div
                          className="preset-management"
                          onClick={(event) => {
                            if (
                              event.target instanceof
                              HTMLButtonElement
                            ) {
                              event.currentTarget
                                .closest("details")
                                ?.removeAttribute(
                                  "open",
                                );
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
                    )}
                  </div>
                );
              },
            )}
          </div>
        </div>
      )}

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
        Favorites ·{" "}
        {formatHotkey(
          hotkeySettings.favoriteMode,
        )}
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
          Undo&nbsp;&nbsp;
          {formatHotkey(
            hotkeySettings.undo,
          )}
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
          Redo&nbsp;&nbsp;
          {formatHotkey(
            hotkeySettings.redo,
          )}
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
                {metadata?.color &&
                  slotColorDisplayMode !== "dot" && (
                    <span
                      className={`slot-card-accent ${slotColorDisplayMode}`}
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
                  title={
                    position && metadata?.screenshot
                      ? "Right click to view screenshot"
                      : undefined
                  }
                  onContextMenu={(event) => {
                    if (
                      !position ||
                      !metadata?.screenshot
                    ) {
                      return;
                    }

                    event.preventDefault();

                    setScreenshotViewer({
                      name: displayName,
                      thumbnailPath:
                        metadata.screenshot,
                    });
                  }}
                >
                  <span className="slot-preview-number">
                    {String(slot).padStart(2, "0")}
                  </span>

                  {position ? (
                    <>
                      {metadata?.screenshot ? (
                        <img
                          className="slot-preview-image"
                          src={convertFileSrc(
                            metadata.screenshot,
                          )}
                          alt=""
                          draggable={false}
                          onLoad={(event) => {
                            event.currentTarget.dataset.retryAttempts =
                              "0";
                          }}
                          onError={(event) => {
                            const image =
                              event.currentTarget;

                            const attempts =
                              Number(
                                image.dataset
                                  .retryAttempts ?? "0",
                              );

                            /*
                            * Le thumbnail peut être encore
                            * en cours d'encodage au moment où
                            * React essaye de l'afficher.
                            *
                            * On retente pendant 2 secondes max.
                            */
                            if (attempts >= 20) {
                              return;
                            }

                            image.dataset.retryAttempts =
                              String(attempts + 1);

                            const screenshotPath =
                              metadata.screenshot!;

                            window.setTimeout(() => {
                              if (!image.isConnected) {
                                return;
                              }

                              image.src =
                                `${convertFileSrc(
                                  screenshotPath,
                                )}?retry=${Date.now()}`;
                            }, 100);
                          }}
                        />
                      ) : (
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
                      )}

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


                        <button
                          className="slot-menu-copy-position"
                          type="button"
                          onClick={async (event) => {
                            event.currentTarget
                              .closest("details")
                              ?.removeAttribute("open");

                            await navigator.clipboard.writeText(
                              `setpos_exact ${position.x.toFixed(2)} ${position.y.toFixed(2)} ${position.z.toFixed(2)}; setang_exact ${position.pitch.toFixed(2)} ${position.yaw.toFixed(2)} ${position.roll.toFixed(2)}`,
                            );
                          }}
                        >
                          Copy position
                        </button>


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

                    {position &&
                      metadata?.screenshot && (
                        <button
                          className="slot-menu-item"
                          type="button"
                          onClick={(event) => {
                            event.currentTarget
                              .closest("details")
                              ?.removeAttribute(
                                "open",
                              );

                            setScreenshotViewer({
                              name: displayName,
                              thumbnailPath:
                                metadata.screenshot!,
                            });
                          }}
                        >
                          View screenshot

                          <span>
                            Right click
                          </span>
                        </button>
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

                  <div className="slot-card-hotkey-group">
                    {metadata?.color &&
                      slotColorDisplayMode === "dot" && (
                        <span
                          className="slot-card-color-dot"
                          style={{
                            backgroundColor:
                              metadata.color,
                          }}
                        />
                      )}

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
        {activeSettingsSection === "general" && (
          <section className="general-settings-section">
            <div className="general-settings-heading">
              <div>
                <p className="label">
                  GENERAL
                </p>

                <h2>
                  General settings
                </h2>

                <p>
                  Configure desktop behavior, Quick
                  Access, appearance and warnings.
                </p>
              </div>
            </div>


          <div className="general-settings-group">
                <h3>APPEARANCE</h3>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Slot color style
                  </strong>

                  <span>
                    Choose how assigned colors are
                    displayed on savestate cards.
                  </span>
                </div>

                <div className="slot-color-style-options">
                  <button
                    className={
                      slotColorDisplayMode === "tint"
                        ? "active"
                        : ""
                    }
                    type="button"
                    onClick={() =>
                      updateSlotColorDisplayMode("tint")
                    }
                  >
                    <span className="slot-color-style-preview tint" />
                    Tint
                  </button>

                  <button
                    className={
                      slotColorDisplayMode === "dot"
                        ? "active"
                        : ""
                    }
                    type="button"
                    onClick={() =>
                      updateSlotColorDisplayMode("dot")
                    }
                  >
                    <span className="slot-color-style-preview dot" />
                    Dot
                  </button>

                  <button
                    className={
                      slotColorDisplayMode === "accent"
                        ? "active"
                        : ""
                    }
                    type="button"
                    onClick={() =>
                      updateSlotColorDisplayMode("accent")
                    }
                  >
                    <span className="slot-color-style-preview accent" />
                    Accent
                  </button>
                </div>
              </div>  


            <div className="general-settings-list">
              <div className="general-settings-group">
                <h3>DESKTOP</h3>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Launch with Windows
                  </strong>

                  <span>
                    Start SPLIT automatically when
                    you sign in to Windows.
                  </span>
                </div>

                <button
                  className={`general-toggle ${
                    autostartEnabled
                      ? "active"
                      : ""
                  }`}
                  type="button"
                  disabled={
                    autostartLoading ||
                    autostartSaving
                  }
                  onClick={() =>
                    void toggleAutostart()
                  }
                >
                  {autostartSaving
                    ? "Saving…"
                    : autostartLoading
                      ? "Checking…"
                      : autostartEnabled
                        ? "On"
                        : "Off"}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Start minimized to tray
                  </strong>

                  <span>
                    {autostartEnabled
                      ? "Keep SPLIT hidden in the system tray when it starts with Windows."
                      : "Enable Launch with Windows to use this option."}
                  </span>
                </div>

                <button
                  className={`general-toggle ${
                    startMinimized
                      ? "active"
                      : ""
                  }`}
                  type="button"
                  disabled={
                    !autostartEnabled ||
                    autostartLoading ||
                    autostartSaving ||
                    startMinimizedLoading ||
                    startMinimizedSaving
                  }
                  onClick={() =>
                    void toggleStartMinimized()
                  }
                >
                  {startMinimizedSaving
                    ? "Saving…"
                    : startMinimizedLoading
                      ? "Checking…"
                      : startMinimized
                        ? "On"
                        : "Off"}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Close button behavior
                  </strong>

                  <span>
                    Choose what happens when you close
                    the SPLIT window.
                  </span>
                </div>

                <div className="general-toggle-group">
                  <button
                    className={`general-toggle ${
                      closeToTray
                        ? "active"
                        : ""
                    }`}
                    type="button"
                    disabled={
                      closeBehaviorLoading ||
                      closeBehaviorSaving
                    }
                    onClick={() =>
                      void updateCloseBehavior(true)
                    }
                  >
                    Minimize to tray
                  </button>

                  <button
                    className={`general-toggle ${
                      !closeToTray
                        ? "active"
                        : ""
                    }`}
                    type="button"
                    disabled={
                      closeBehaviorLoading ||
                      closeBehaviorSaving
                    }
                    onClick={() =>
                      void updateCloseBehavior(false)
                    }
                  >
                    Quit SPLIT
                  </button>
                </div>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Window size & position
                  </strong>

                  <span>
                    Restore the default window size
                    and center SPLIT on the screen.
                  </span>
                </div>

                <button
                  className="general-restore-button"
                  type="button"
                  disabled={resettingWindow}
                  onClick={() =>
                    void resetWindow()
                  }
                >
                  {resettingWindow
                    ? "Resetting…"
                    : "Reset"}
                </button>
              </div>

              </div>

              <div
                className={`general-settings-group quick-access-settings-group ${
                  quickAccessSettings.enabled
                    ? ""
                    : "disabled"
                }`}
              >
                <h3>QUICK ACCESS</h3>

                <div className="general-setting-row">
                  <div>
                    <strong>Enabled</strong>
                    <span>
                      Show the Quick Access overlay in Deadlock.
                    </span>
                  </div>

                  <button
                    className={`general-toggle ${
                      quickAccessSettings.enabled
                        ? "active"
                        : ""
                    }`}
                    type="button"
                    role="switch"
                    aria-checked={quickAccessSettings.enabled}
                    disabled={
                      quickAccessSettingsLoading ||
                      quickAccessSettingsSaving
                    }
                    onClick={() =>
                      void updateQuickAccessSettings({
                        ...quickAccessSettings,
                        enabled: !quickAccessSettings.enabled,
                      })
                    }
                  >
                    {quickAccessSettings.enabled ? "On" : "Off"}
                  </button>
                </div>

                <div className="general-setting-row quick-access-dependent-setting">
                  <div>
                    <strong>Shortcut</strong>
                    <span>Hold shortcut to interact</span>
                  </div>

                  <div className="quick-access-shortcut-control">
                    <kbd>
                      {isHotkeyTarget(
                        capturingHotkey,
                        { group: "quickAccess" },
                      )
                        ? "Press a shortcut..."
                        : formatHotkey(hotkeySettings.quickAccess)}
                    </kbd>
                    <button
                      className="general-restore-button"
                      type="button"
                      disabled={
                        !quickAccessSettings.enabled ||
                        quickAccessSettingsLoading ||
                        quickAccessSettingsSaving ||
                        hotkeySettingsSaving
                      }
                      onClick={() => {
                        const target: HotkeyTarget = {
                          group: "quickAccess",
                        };
                        const capturing = isHotkeyTarget(
                          capturingHotkey,
                          target,
                        );
                        setHotkeyMessage(null);
                        setCapturingHotkey(
                          capturing ? null : target,
                        );
                      }}
                    >
                      {isHotkeyTarget(
                        capturingHotkey,
                        { group: "quickAccess" },
                      )
                        ? "Cancel"
                        : "Change"}
                    </button>
                  </div>
                </div>

                <div className="general-setting-row quick-access-dependent-setting">
                  <div>
                    <strong>Position</strong>
                    <span>
                      Anchor the overlay inside the Deadlock window.
                    </span>
                  </div>

                  <div className="general-toggle-group">
                    {(["left", "right"] as const).map(
                      (position) => (
                        <button
                          key={position}
                          className={`general-toggle ${
                            quickAccessSettings.position === position
                              ? "active"
                              : ""
                          }`}
                          type="button"
                          disabled={
                            !quickAccessSettings.enabled ||
                            quickAccessSettingsLoading ||
                            quickAccessSettingsSaving
                          }
                          onClick={() =>
                            void updateQuickAccessSettings({
                              ...quickAccessSettings,
                              position,
                            })
                          }
                        >
                          {position === "left" ? "Left" : "Right"}
                        </button>
                      ),
                    )}
                  </div>
                </div>

                {hotkeyMessage && (
                  <p className="hotkey-message" role="alert">
                    {hotkeyMessage}
                  </p>
                )}
              </div>

              

              </div>

              <div className="general-settings-group">
                <h3>CONFIRMATIONS &amp; WARNINGS</h3>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Clear preset confirmation
                  </strong>

                  <span>
                    Show the confirmation dialog
                    before clearing an entire preset.
                  </span>
                </div>

                <button
                  className="general-restore-button"
                  type="button"
                  onClick={
                    restoreClearPresetConfirmation
                  }
                >
                  {clearPresetConfirmationRestored
                    ? "Restored"
                    : "Restore confirmation"}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Gameplay hotkey warning
                  </strong>

                  <span>
                    Warn when a shortcut may interfere
                    with Deadlock controls.
                  </span>
                </div>

                <button
                  className="general-restore-button"
                  type="button"
                  onClick={
                    restoreGameplayHotkeyWarning
                  }
                >
                  {gameplayHotkeyWarningRestored
                    ? "Restored"
                    : "Restore warning"}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    Favorite Mode warning
                  </strong>

                  <span>
                    Show the information dialog when
                    entering Favorite Mode.
                  </span>
                </div>

                <button
                  className="general-restore-button"
                  type="button"
                  onClick={
                    restoreFavoriteModeWarning
                  }
                >
                  {favoriteModeWarningRestored
                    ? "Restored"
                    : "Restore warning"}
                </button>
              </div>
              </div>
            </div>
          </section>
        )}

    {activeSettingsSection === "hotkeys" && (    
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
          Customize the shortcuts used while Deadlock is focused.
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
    )}
    

    {activeSettingsSection === "notifications" && (
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
    )}  

    {activeSettingsSection === "diagnostics" && ( 
      <section
        className="status-grid"
        aria-label="Deadlock diagnostics"
      >
        <article
          className={`status-card ${
            status.integrationHealthy
              ? ""
              : "wide diagnostic-card"
          }`}
        >
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
                    Enter "Explore NYC".
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

        

        <article
          className={`status-card wide ${
            status.cameraRuntimeChecked &&
            !status.cameraRuntimeReady
              ? "diagnostic-card"
              : ""
          }`}
        >
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
    )}  
      </div>
      )}

      {error && (
        <div className="error-box">
          Backend error: {error}
        </div>
      )}

      
      </main>
    </div>
  );
}

export default App;
