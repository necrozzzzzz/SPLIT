import {
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  convertFileSrc,
  invoke,
} from "@tauri-apps/api/core";

import {
  getVersion,
} from "@tauri-apps/api/app";

import {
  listen,
  emit,
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

import {
  isBorderlessEdition,
  splitEditionLabel,
} from "./edition";

import {
  claimStartupSound,
  playClaimedStartupSound,
  playStartupSound,
  type StartupSoundSettings,
} from "./startupSound";

import DiscordPresenceSettings from "./DiscordPresenceSettings";
import { getLanguage, setLanguage, languages, languageNames, t, uiText, diagnosticText, runtimeErrorText, runtimeUiText, type Language } from "./i18n";
import Onboarding, { isOnboardingCompleted } from "./Onboarding";

type DeadlockStatus = {
  deadlockRunning: boolean;
  matchSafetyLocked: boolean;
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
  invalidConfiguredPath: boolean;
};

type LaunchFolderState = {
  configuredPath: string | null;
  pathValid: boolean;
  reminderDue: boolean;
};

const V1_HEADER_PHRASES = [
  "The Lash approves",
  "Go play Makrill blyat",
  "Expelling goo",
  "Haze is missing",
  "Arigato, Arighetto, uh-REG-eh-doe",
  "Help me deliver this thing",
  "They're in mid",
  "Let's go mid",
  "Yellow needs help",
  "Broadway needs help",
  "Nebraska is on its way",
  "I like my pillow",
  "Turn into frog",
  "Why does bread taste good?",
] as const;

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
  useSlotColor: boolean;
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
  movementRecord: Hotkey;
  movementRace: Hotkey;
  movementReplay: Hotkey;
  movementRetry: Hotkey;
  quickAccess: Hotkey;
};

type QuickAccessSettings = {
  enabled: boolean;
  position: "left" | "right";
};

type HotkeyTarget =
  | { group: "loadSlots" | "saveSlots"; index: number }
  | { group: "undo" | "redo" | "cyclePreset" | "favoriteMode" | "movementRecord" | "movementRace" | "movementReplay" | "movementRetry" | "quickAccess" };

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
  useSlotColor: true,
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
  cyclePreset: { key: "F12", ctrl: false, alt: false, shift: false },
  favoriteMode: { key: "F11", ctrl: false, alt: false, shift: false },
  movementRecord: { key: "R", ctrl: true, alt: false, shift: true },
  movementRace: { key: "T", ctrl: true, alt: false, shift: true },
  movementReplay: { key: "P", ctrl: true, alt: false, shift: true },
  movementRetry: { key: "Y", ctrl: true, alt: false, shift: true },
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

const DEFAULT_STARTUP_SOUND_SETTINGS: StartupSoundSettings = {
  enabled: true,
  volume: 40,
};

function formatHotkey(hotkey: Hotkey): string {
  return [
    hotkey.ctrl ? "Ctrl" : null,
    hotkey.alt ? "Alt" : null,
    hotkey.shift ? "Shift" : null,
    hotkey.key,
  ].filter(Boolean).join(" + ");
}

function HotkeyKeys({
  hotkey,
  capturing = false,
}: {
  hotkey: Hotkey;
  capturing?: boolean;
}) {
  if (capturing) {
    return (
      <kbd className="hotkey-binding capturing">
        {uiText("Press a shortcut…", getLanguage())}
      </kbd>
    );
  }

  const keys = [
    hotkey.ctrl ? "Ctrl" : null,
    hotkey.alt ? "Alt" : null,
    hotkey.shift ? "Shift" : null,
    hotkey.key,
  ].filter(Boolean) as string[];

  return (
    <kbd
      className="hotkey-binding"
      aria-label={`${uiText("Shortcut", getLanguage())}: ${formatHotkey(hotkey)}`}
    >
      {keys.map((key, index) => (
        <span
          className="hotkey-key-part"
          key={`${key}-${index}`}
        >
          {index > 0 && (
            <span
              className="hotkey-key-plus"
              aria-hidden="true"
            >
              +
            </span>
          )}

          <span className="hotkey-keycap">
            {key}
          </span>
        </span>
      ))}
    </kbd>
  );
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
  language: Language,
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
      return `${runtimeUiText("Load Slot", language)} ${index + 1}`;
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
      return `${runtimeUiText("Save Slot", language)} ${index + 1}`;
    }
  }

  if (
    target.group !== "undo" &&
    hotkeysEqual(
      settings.undo,
      candidate,
    )
  ) {
    return runtimeUiText("Undo", language);
  }

  if (
    target.group !== "redo" &&
    hotkeysEqual(
      settings.redo,
      candidate,
    )
  ) {
    return runtimeUiText("Redo", language);
  }

  if (
    target.group !== "cyclePreset" &&
    hotkeysEqual(
      settings.cyclePreset,
      candidate,
    )
  ) {
    return runtimeUiText("Cycle Preset", language);
  }

  if (
    target.group !== "favoriteMode" &&
    hotkeysEqual(
      settings.favoriteMode,
      candidate,
    )
  ) {
    return runtimeUiText("Favorite Mode", language);
  }

  for (const [group, label] of [
    ["movementRecord", "Movement Record"],
    ["movementRace", "Movement Race"],
    ["movementReplay", "Movement Replay"],
    ["movementRetry", "Movement Retry"],
  ] as const) {
    if (target.group !== group && hotkeysEqual(settings[group], candidate)) {
      return runtimeUiText(label, language);
    }
  }

  if (
    target.group !== "quickAccess" &&
    hotkeysEqual(
      settings.quickAccess,
      candidate,
    )
  ) {
    return runtimeUiText("Quick Access", language);
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

function isSimpleTextHotkey(hotkey: Hotkey): boolean {
  return (
    !hotkey.ctrl &&
    !hotkey.alt &&
    !hotkey.shift &&
    /^[A-Z0-9]$/i.test(hotkey.key)
  );
}

const EMPTY_STATUS: DeadlockStatus = {
  deadlockRunning: false,
  matchSafetyLocked: false,
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
  | "info"
  | "warning"
  | "error"
  | "off";

type AppView = "slots" | "movement" | "settings";

type MovementVisualMode = "both" | "ghost" | "tracer" | "off";
type MovementDisplaySettings = {
  visual: MovementVisualMode;
  rings: boolean;
  checkpoints: boolean;
  hud: boolean;
};
const MOVEMENT_DISPLAY_KEY = "split.movement.display.v1";
const DEFAULT_MOVEMENT_DISPLAY: MovementDisplaySettings = {
  visual: "both",
  rings: true,
  checkpoints: true,
  hud: true,
};
function readMovementDisplay(): MovementDisplaySettings {
  try {
    const saved = JSON.parse(localStorage.getItem(MOVEMENT_DISPLAY_KEY) ?? "null");
    if (!saved || typeof saved !== "object") return { ...DEFAULT_MOVEMENT_DISPLAY };
    return {
      visual: ["both", "ghost", "tracer", "off"].includes(saved.visual) ? saved.visual : "both",
      rings: typeof saved.rings === "boolean" ? saved.rings : true,
      checkpoints: typeof saved.checkpoints === "boolean" ? saved.checkpoints : true,
      hud: typeof saved.hud === "boolean" ? saved.hud : true,
    };
  } catch {
    return { ...DEFAULT_MOVEMENT_DISPLAY };
  }
}

type MovementLabStatus = {
  version: string;
  playerConnected: boolean;
  humanSlot: number;
  recording: boolean;
  ticks: number;
  hasRoute: boolean;
  heroId: number;
  racePhase: "idle" | "armed" | "running" | "finished";
  elapsedSeconds: number;
  lastSeconds: number | null;
  personalBestSeconds: number | null;
  lastWasPb: boolean;
  lastDeltaSeconds: number | null;
  lastSplitDeltas: number[];
  currentSplits: number[];
  personalBestSplits: number[];
  nextCheckpoint: number;
  checkpointCount: number;
  visualMode?: MovementVisualMode;
  showRings?: boolean;
  showCheckpoints?: boolean;
  showHud?: boolean;
  practiceInfiniteStamina?: boolean;
  practiceNoCooldowns?: boolean;
  practiceInfiniteAmmo?: boolean;
};

type SlotColorDisplayMode =
  | "tint"
  | "accent"
  | "dot";

type SettingsSection =
  | "general"
  | "discord"
  | "hotkeys"
  | "notifications"
  | "diagnostics"
  | "about";

function formatSavedAge(
  savedAt: number | null,
  nowMs: number,
  language: Language,
): string | null {
  if (savedAt === null) return null;
  const seconds = Math.max(0, Math.floor(nowMs / 1000) - savedAt);
  const locale = language === "pt" ? "pt-BR" : language === "zh" ? "zh-CN" : language;
  const saved = runtimeUiText("Saved", language);
  if (seconds < 60) {
    return `${saved} · ${new Intl.RelativeTimeFormat(locale, { numeric: "auto" }).format(0, "second")}`;
  }
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: "always", style: "short" });
  if (minutes < 60) return `${saved} ${relative.format(-minutes, "minute")}`;
  if (hours < 24) return `${saved} ${relative.format(-hours, "hour")}`;
  if (days < 7) return `${saved} ${relative.format(-days, "day")}`;
  return `${saved} ${new Date(savedAt * 1000).toLocaleDateString(locale)}`;
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

function DiagnosticInfo({ text, fix, language = "en" }: { text: string; fix?: string; language?: Language }) {
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const close = (e: PointerEvent) => { if (ref.current?.open && !ref.current.contains(e.target as Node)) ref.current.open = false; };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, []);
  return (
    <details ref={ref} className="diagnostic-info">
      <summary aria-label={t("whatDoesItMean", language)} title={t("whatDoesItMean", language)}>i</summary>
      <div className="diagnostic-info-content"><div className="diagnostic-info-label">{t("meaning", language).toUpperCase()}</div><p>{diagnosticText(text, language)}</p>{fix && <><div className="diagnostic-info-separator" /><div className="diagnostic-info-label">{t("troubleshooting", language).toUpperCase()}</div><p>{diagnosticText(fix, language)}</p></>}</div>
    </details>
  );
}

function App() {
  const [language, updateLanguage] = useState<Language>(getLanguage);
  const [automaticLanguage, setAutomaticLanguage] = useState(() => localStorage.getItem("split.language") === null);
  const changeLanguage = (value: string) => {
    if (value === "auto") {
      localStorage.removeItem("split.language");
      setAutomaticLanguage(true);
      updateLanguage(getLanguage());
      window.dispatchEvent(new Event("split-language-change"));
    } else {
      setLanguage(value as Language);
      updateLanguage(value as Language);
      setAutomaticLanguage(false);
    }
  };
  useEffect(() => { document.documentElement.lang = language; }, [language]);
  useEffect(() => { void invoke("set_notification_language", { language }).catch(() => undefined); }, [language]);
  useEffect(() => { void emit("split-language-updated", language).catch(() => undefined); }, [language]);
  const [showOnboarding, setShowOnboarding] = useState(() => !isOnboardingCompleted());
  const [activeView, setActiveView] =
    useState<AppView>("slots");
  const [movementStatus, setMovementStatus] = useState<MovementLabStatus | null>(null);
  const [movementBusy, setMovementBusy] = useState<string | null>(null);
  const [movementError, setMovementError] = useState<string | null>(null);
  const [movementDisplay, setMovementDisplay] = useState<MovementDisplaySettings>(readMovementDisplay);
  const [movementDisplaySaving, setMovementDisplaySaving] = useState(false);
  const updateMovementDisplay = useCallback((patch: Partial<MovementDisplaySettings>) => {
    setMovementDisplay((previous) => {
      const next = { ...previous, ...patch };
      localStorage.setItem(MOVEMENT_DISPLAY_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const refreshMovementStatus = useCallback(async () => {
    try {
      const next = await invoke<MovementLabStatus>("get_movement_lab_status");
      setMovementStatus(next);
      setMovementError(null);
    } catch (reason) {
      setMovementStatus(null);
      setMovementError(String(reason));
    }
  }, []);

  // Synchronize on connect and on preference changes; status polling alone
  // must not flood the localhost control socket.
  useEffect(() => {
    if (!movementStatus) return;
    const actual = [
      movementStatus.visualMode ?? "both",
      movementStatus.showRings ?? true,
      movementStatus.showCheckpoints ?? true,
      movementStatus.showHud ?? true,
    ];
    const wanted = [movementDisplay.visual, movementDisplay.rings, movementDisplay.checkpoints, movementDisplay.hud];
    if (actual.every((value, index) => value === wanted[index])) return;
    let cancelled = false;
    setMovementDisplaySaving(true);
    const commands = [
      `visual ${movementDisplay.visual}`,
      `rings ${movementDisplay.rings ? "on" : "off"}`,
      `checkpoints ${movementDisplay.checkpoints ? "on" : "off"}`,
      `hud ${movementDisplay.hud ? "on" : "off"}`,
    ];
    void (async () => {
      try {
        for (const command of commands) {
          if (cancelled) break;
          await invoke("movement_lab_command", { command });
        }
        if (!cancelled) await refreshMovementStatus();
      } catch (reason) {
        if (!cancelled) setMovementError(String(reason));
      } finally {
        if (!cancelled) setMovementDisplaySaving(false);
      }
    })();
    return () => { cancelled = true; };
  }, [movementStatus?.version, movementDisplay.visual, movementDisplay.rings, movementDisplay.checkpoints, movementDisplay.hud, refreshMovementStatus]);

  const runMovementCommand = useCallback(async (command: string) => {
    setMovementBusy(command);
    setMovementError(null);
    try {
      await invoke("movement_lab_command", { command });
      window.setTimeout(() => void refreshMovementStatus(), 80);
    } catch (reason) {
      setMovementError(String(reason));
    } finally {
      setMovementBusy(null);
    }
  }, [refreshMovementStatus]);

  useEffect(() => {
    if (activeView !== "movement") return;
    void refreshMovementStatus();
    const timer = window.setInterval(() => void refreshMovementStatus(), 250);
    return () => window.clearInterval(timer);
  }, [activeView, refreshMovementStatus]);

  const [
    activeSettingsSection,
    setActiveSettingsSection,
  ] = useState<SettingsSection>(
    "general",
  );

  const navigateOnboarding = useCallback((view: "slots" | "settings", section?: SettingsSection) => {
    setActiveView(view);
    if (section) setActiveSettingsSection(section);
  }, []);

  const [appVersion, setAppVersion] =
    useState<string | null>(null);

  useEffect(() => {
    let active = true;

    void getVersion()
      .then((version) => {
        if (active) {
          setAppVersion(version);
        }
      })
      .catch(() => {
        if (active) {
          setAppVersion(runtimeUiText("Unavailable", getLanguage()));
        }
      });

    return () => {
      active = false;
    };
  }, []);


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

  const [startupSoundSettings, setStartupSoundSettings] =
    useState<StartupSoundSettings>(DEFAULT_STARTUP_SOUND_SETTINGS);
  const [startupSoundSaving, setStartupSoundSaving] = useState(false);
  const [startupSoundFeedback, setStartupSoundFeedback] =
    useState<string | null>(null);
  const startupSoundSaveTimerRef = useRef<number | undefined>(undefined);
  const startupSoundSaveSequenceRef = useRef(0);
  const startupSoundSaveQueueRef = useRef<Promise<void>>(Promise.resolve());

  const [focusDeadlockOnStartup, setFocusDeadlockOnStartup] = useState(false);
  const [focusDeadlockOnStartupLoading, setFocusDeadlockOnStartupLoading] =
    useState(true);
  const [focusDeadlockOnStartupSaving, setFocusDeadlockOnStartupSaving] =
    useState(false);

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
    pendingPresetRename,
    setPendingPresetRename,
  ] = useState<{
    preset: number;
    currentName: string;
    value: string;
    error: string | null;
  } | null>(null);

  const presetRenameInputRef =
    useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!pendingPresetRename) {
      return;
    }

    presetRenameInputRef.current?.focus();
    presetRenameInputRef.current?.select();
  }, [pendingPresetRename?.preset]);

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

  const [folderReminder, setFolderReminder] =
    useState<LaunchFolderState | null>(null);

  const [v1PhraseIndex, setV1PhraseIndex] =
    useState(() => Math.floor(Math.random() * V1_HEADER_PHRASES.length));
  const [v1PhrasePhase, setV1PhrasePhase] =
    useState<"visible" | "exiting" | "entering">("visible");

  const [notificationTesting, setNotificationTesting] =
    useState(false);
  const [
    notificationTestFeedback,
    setNotificationTestFeedback,
  ] = useState<{
    error: boolean;
    text: string;
  } | null>(null);

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
    editingSlot,
    setEditingSlot,
  ] = useState<number | null>(null);

  const [
    editingSlotName,
    setEditingSlotName,
  ] = useState("");

  const slotRenameInputRef =
    useRef<HTMLInputElement | null>(null);

  const committingSlotRenameRef =
    useRef<number | null>(null);


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
    diagnosticSummaryExpanded,
    setDiagnosticSummaryExpanded,
  ] = useState(false);

  const [
    error,
    setError,
  ] =
    useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let changeTimer: number | undefined;
    let transitionTimer: number | undefined;
    let enterFrame: number | undefined;
    let settleFrame: number | undefined;

    const scheduleChange = () => {
      changeTimer = window.setTimeout(() => {
        setV1PhrasePhase("exiting");
        transitionTimer = window.setTimeout(() => {
          if (disposed) return;
          setV1PhraseIndex((current) => {
            const offset = 1 + Math.floor(
              Math.random() * (V1_HEADER_PHRASES.length - 1),
            );
            return (current + offset) % V1_HEADER_PHRASES.length;
          });
          setV1PhrasePhase("entering");
          enterFrame = window.requestAnimationFrame(() => {
            settleFrame = window.requestAnimationFrame(() => {
              if (!disposed) setV1PhrasePhase("visible");
            });
          });
          scheduleChange();
        }, 280);
      }, 8_000 + Math.floor(Math.random() * 4_001));
    };

    scheduleChange();
    return () => {
      disposed = true;
      if (changeTimer !== undefined) window.clearTimeout(changeTimer);
      if (transitionTimer !== undefined) window.clearTimeout(transitionTimer);
      if (enterFrame !== undefined) window.cancelAnimationFrame(enterFrame);
      if (settleFrame !== undefined) window.cancelAnimationFrame(settleFrame);
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<boolean>(
      "deadlock-match-safety-changed",
      (event) => {
        if (!disposed) {
          setStatus((current) => ({
            ...current,
            matchSafetyLocked: event.payload,
          }));
        }
      },
    ).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;

    void claimStartupSound()
      .then((launch) => {
        if (!disposed) {
          setStartupSoundSettings(launch.settings);
        }
        void playClaimedStartupSound(launch);
      })
      .catch(() => {
        // Startup audio must never block or surface a critical error.
      });

    return () => {
      disposed = true;
    };
  }, []);

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
            `${runtimeErrorText("Could not read startup setting: ", getLanguage())}${String(reason)}`,
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

    void invoke<boolean>("get_focus_deadlock_on_startup")
      .then((enabled) => {
        if (!disposed) setFocusDeadlockOnStartup(enabled);
      })
      .catch((reason) => {
        if (!disposed) {
          setError(`${runtimeErrorText("Could not read Deadlock startup focus setting: ", getLanguage())}${String(reason)}`);
        }
      })
      .finally(() => {
        if (!disposed) setFocusDeadlockOnStartupLoading(false);
      });

    return () => {
      disposed = true;
    };
  }, []);

  const toggleFocusDeadlockOnStartup = useCallback(async () => {
    const next = !focusDeadlockOnStartup;
    setFocusDeadlockOnStartupSaving(true);
    setError(null);

    try {
      const saved = await invoke<boolean>("update_focus_deadlock_on_startup", {
        enabled: next,
      });
      setFocusDeadlockOnStartup(saved);
    } catch (reason) {
      setError(`${runtimeErrorText("Could not update Deadlock startup focus setting: ", getLanguage())}${String(reason)}`);
    } finally {
      setFocusDeadlockOnStartupSaving(false);
    }
  }, [focusDeadlockOnStartup]);

  const persistStartupSoundSettings = useCallback(
    (next: StartupSoundSettings) => {
      const sequence = ++startupSoundSaveSequenceRef.current;
      setStartupSoundSaving(true);
      setStartupSoundFeedback(null);

      const save = async () => {
        try {
          const saved = await invoke<StartupSoundSettings>(
            "update_startup_sound_settings",
            { settings: next },
          );
          if (sequence === startupSoundSaveSequenceRef.current) {
            setStartupSoundSettings(saved);
          }
        } catch (reason) {
          if (sequence === startupSoundSaveSequenceRef.current) {
            setStartupSoundFeedback(
              `${runtimeUiText("Could not save startup sound settings: ", getLanguage())}${String(reason)}`,
            );
          }
        } finally {
          if (sequence === startupSoundSaveSequenceRef.current) {
            setStartupSoundSaving(false);
          }
        }
      };

      startupSoundSaveQueueRef.current =
        startupSoundSaveQueueRef.current.then(save, save);
    },
    [],
  );

  const scheduleStartupSoundVolumeSave = useCallback(
    (next: StartupSoundSettings) => {
      if (startupSoundSaveTimerRef.current !== undefined) {
        window.clearTimeout(startupSoundSaveTimerRef.current);
      }
      startupSoundSaveTimerRef.current = window.setTimeout(() => {
        startupSoundSaveTimerRef.current = undefined;
        persistStartupSoundSettings(next);
      }, 250);
    },
    [persistStartupSoundSettings],
  );

  const flushStartupSoundVolumeSave = useCallback(
    (next: StartupSoundSettings) => {
      if (startupSoundSaveTimerRef.current === undefined) return;
      window.clearTimeout(startupSoundSaveTimerRef.current);
      startupSoundSaveTimerRef.current = undefined;
      persistStartupSoundSettings(next);
    },
    [persistStartupSoundSettings],
  );

  const previewStartupSound = useCallback(async () => {
    setStartupSoundFeedback(null);
    try {
      await playStartupSound(startupSoundSettings.volume);
    } catch (reason) {
      setStartupSoundFeedback(
        `${runtimeUiText("Could not play the startup sound: ", getLanguage())}${String(reason)}`,
      );
    }
  }, [startupSoundSettings.volume]);

  const toggleStartupSound = useCallback(() => {
    if (startupSoundSaveTimerRef.current !== undefined) {
      window.clearTimeout(startupSoundSaveTimerRef.current);
      startupSoundSaveTimerRef.current = undefined;
    }
    const next = {
      ...startupSoundSettings,
      enabled: !startupSoundSettings.enabled,
    };
    setStartupSoundSettings(next);
    persistStartupSoundSettings(next);
  }, [persistStartupSoundSettings, startupSoundSettings]);

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
            `${runtimeErrorText("Could not read minimized startup setting: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not update startup setting: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not update minimized startup setting: ", getLanguage())}${String(reason)}`,
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
            `${runtimeErrorText("Could not read close behavior: ", getLanguage())}${String(reason)}`,
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
            `${runtimeErrorText("Could not update close behavior: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not reset window: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not launch Deadlock: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not copy diagnostic report: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not prepare teleport points: ", getLanguage())}${String(reason)}`,
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
          `${runtimeErrorText("Could not resume Deadlock presentation: ", getLanguage())}${String(reason)}`,
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
        try {
          const launchState =
            await invoke<LaunchFolderState>(
              "record_successful_launch",
            );

          if (!disposed && launchState.reminderDue) {
            setFolderReminder(launchState);
          }
        } catch (reason) {
          /*
           * Updating lastLaunchAt is useful but must never prevent
           * the setup state from resolving. In particular, a damaged
           * config must fall through to auto-detection/manual choice.
           */
          if (!disposed) {
            setError(String(reason));
          }
        }

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
          void invoke("report_startup_error", {
            message: `get_deadlock_setup failed: ${String(reason)}`,
          });
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
    let unlisten: (() => void) | undefined;

    void listen<DeadlockStatus>(
      "deadlock-status-changed",
      (event) => {
        if (!disposed) setStatus(event.payload);
      },
    ).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
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

      useEffect(() => {
        if (editingSlot === null) {
          return;
        }

        slotRenameInputRef.current?.focus();
        slotRenameInputRef.current?.select();
      }, [editingSlot]);

      const beginSlotRename =
        useCallback(
          (
            slot: number,
            currentName: string,
          ) => {
            if (
              savingSlot !== null ||
              loadingSlot !== null
            ) {
              return;
            }

            setError(null);
            setEditingSlotName(currentName);
            setEditingSlot(slot);
          },
          [loadingSlot, savingSlot],
        );

      const finishSlotRename =
        useCallback(
          async (
            slot: number,
            previousName: string,
            save: boolean,
          ) => {
            if (
              committingSlotRenameRef.current !==
              null
            ) {
              return;
            }

            const trimmed =
              editingSlotName.trim();

            committingSlotRenameRef.current =
              slot;
            setEditingSlot(null);

            try {
              if (!save) {
                setEditingSlotName(previousName);
                return;
              }

              if (!trimmed) {
                setEditingSlotName(previousName);
                setError(
                  runtimeErrorText("Slot name cannot be empty", getLanguage()),
                );
                return;
              }

              if (
                trimmed === previousName.trim()
              ) {
                setEditingSlotName(previousName);
                return;
              }

              setError(null);
              const result =
                await invoke<SlotEditResult>(
                  "rename_slot",
                  {
                    slot,
                    name: trimmed,
                  },
                );

              await applySlotEditResult(result);
            } catch (reason) {
              setError(String(reason));
            } finally {
              window.setTimeout(() => {
                if (
                  committingSlotRenameRef.current ===
                  slot
                ) {
                  committingSlotRenameRef.current =
                    null;
                }
              }, 0);
            }
          },
          [
            applySlotEditResult,
            editingSlotName,
          ],
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
          () => {
            const currentName =
              presetNames[
                activePreset - 1
              ] ??
              `Preset ${activePreset}`;

            setPendingPresetRename({
              preset: activePreset,
              currentName,
              value: currentName,
              error: null,
            });
          },
          [
            activePreset,
            presetNames,
          ],
        );


      const submitPresetRename =
        useCallback(
          async () => {
            if (!pendingPresetRename || renamingPreset) {
              return;
            }

            const trimmed = pendingPresetRename.value.trim();

            if (!trimmed) {
              setPendingPresetRename((current) =>
                current
                  ? {
                      ...current,
                      error: runtimeUiText("Preset name cannot be empty", language),
                    }
                  : current,
              );
              return;
            }

            setRenamingPreset(true);
            setPendingPresetRename((current) =>
              current
                ? { ...current, error: null }
                : current,
            );

            try {
              const names =
                await invoke<
                  Array<string>
                >(
                  "rename_preset",
                  {
                    preset:
                      pendingPresetRename.preset,
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
              setPendingPresetRename(null);
            } catch (reason) {
              setPendingPresetRename((current) =>
                current
                  ? {
                      ...current,
                      error: String(reason),
                    }
                  : current,
              );
            } finally {
              setRenamingPreset(
                false,
              );
            }
          },
          [
            pendingPresetRename,
            renamingPreset,
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
                  : runtimeUiText("Invalid preset", language);

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
            `${runtimeErrorText("Could not load Favorites: ", getLanguage())}${String(reason)}`,
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
            `${runtimeErrorText("Could not save to Favorites: ", getLanguage())}${String(reason)}`,
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

  const testNotification = useCallback(async () => {
    if (notificationTesting) {
      return;
    }

    setNotificationTesting(true);
    setNotificationTestFeedback(null);

    try {
      await invoke("test_notification");
      setNotificationTestFeedback({
        error: false,
        text: "Test notification sent.",
      });
    } catch (reason) {
      setNotificationTestFeedback({
        error: true,
        text: String(reason),
      });
    } finally {
      setNotificationTesting(false);
    }
  }, [notificationTesting]);

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
          language,
        );

      if (conflictLabel) {
        setHotkeyMessage(
          `${formatHotkey(hotkey)}${runtimeUiText(" is already assigned to ", language)}${conflictLabel}${runtimeUiText(".", language)}`,
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
    [hotkeySettings, language],
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
            invalidConfiguredPath: false,
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
            runtimeUiText("Choose the Deadlock installation folder", language),
        });

      if (
        selected === null ||
        Array.isArray(selected)
      ) {
        return;
      }

      await confirmPath(selected);
    }, [confirmPath]);

  const chooseReminderFolder =
    useCallback(async () => {
      setError(null);
      const selected = await open({
        directory: true,
        multiple: false,
        title: runtimeUiText("Choose the Deadlock installation folder", language),
      });
      if (selected === null || Array.isArray(selected)) return;

      setSetupWorking(true);
      try {
        const nextStatus = await invoke<DeadlockStatus>(
          "confirm_deadlock_path",
          { path: selected },
        );
        setStatus(nextStatus);
        setSetup({
          configuredPath: selected,
          detectedPath: null,
          needsSetup: false,
          invalidConfiguredPath: false,
        });
        setFolderReminder(null);
      } catch (reason) {
        setError(String(reason));
      } finally {
        setSetupWorking(false);
      }
    }, []);

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
          invalidConfiguredPath: false,
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
              SPLIT
            </p>

            <div className="setup-loading-heading">
              <div className="setup-loading-icon">
                <span />
              </div>

              <div>
                <h1>
                  {uiText("Detecting Deadlock", language)}
                </h1>

                <p className="setup-description">
                  {runtimeUiText("Scanning your Steam libraries for a Deadlock installation.", language)}
                </p>
              </div>
            </div>

            <div className="setup-scan-status">
              <div className="setup-scan-status-header">
                <span>{runtimeUiText("INSTALLATION SCAN", language)}</span>
                <strong>{uiText("Searching…", language)}</strong>
              </div>

              <div className="setup-scan-track">
                <span />
              </div>

              <p>
                {runtimeUiText("Checking Steam libraries and known installation locations.", language)}
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
      setup.invalidConfiguredPath ? null : setup.detectedPath;

    return (
      <main className="shell setup-shell">
        <section className="setup-card setup-onboarding-card">
          <div className="setup-onboarding-grid">
            <div className="setup-onboarding-main">
              <p className="eyebrow">
                {runtimeUiText("SPLIT · FIRST SETUP", language)}
              </p>

              <h1>
                {uiText("Deadlock installation", language)}
              </h1>

              {detected ? (
                <>
                  <p className="setup-description">
                    {runtimeUiText("SPLIT detected a Deadlock installation.", language)}
                  </p>

                  <div className="detected-folder">
                    <span>
                      {runtimeUiText("DETECTED FOLDER", language)}
                    </span>

                    <code>
                      {detected}
                    </code>
                  </div>

                  <h2 className="setup-question">
                    {runtimeUiText("Is this the correct Deadlock folder?", language)}
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
                        ? runtimeUiText("Configuring…", language)
                        : runtimeUiText("Yes, continue", language)}
                    </button>

                    <button
                      className="secondary-button"
                      type="button"
                      disabled={setupWorking}
                      onClick={() =>
                        void chooseFolder()
                      }
                    >
                      {uiText("No, choose folder", language)}
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p className="setup-description">
                    {setup.invalidConfiguredPath
                      ? runtimeUiText("Your configured Deadlock folder is no longer valid. Choose the current installation folder to continue.", language)
                      : runtimeUiText("SPLIT couldn't detect a Deadlock installation in your Steam libraries.", language)}
                  </p>

                  {setup.invalidConfiguredPath && setup.configuredPath && (
                    <div className="detected-folder invalid-folder">
                      <span>{runtimeUiText("INVALID CONFIGURED FOLDER", language)}</span>
                      <code>{setup.configuredPath}</code>
                    </div>
                  )}

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
                      <span>{runtimeUiText("MANUAL SELECTION", language)}</span>

                      <strong>
                        {uiText("Select your Deadlock folder", language)}
                      </strong>

                      <p>
                        {runtimeUiText("Choose the main installation directory, usually located inside:", language)}
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
                      {uiText("Choose Deadlock folder", language)}
                    </button>

                    <button
                      className="secondary-button"
                      type="button"
                      disabled={setupWorking}
                      onClick={() => void rescan()}
                    >
                      {uiText("Scan again", language)}
                    </button>
                  </div>
                </>
              )}

              {setupWorking && (
                <p className="setup-working">
                  {uiText("Setting up SPLIT integration…", language)}
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
              aria-label={uiText("Setup progress", getLanguage())}
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
                    {uiText("Deadlock detection", language)}
                  </strong>

                  <small>
                    {detected
                      ? runtimeUiText("Installation found", language)
                      : runtimeUiText("Not detected automatically", language)}
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
                      ? runtimeUiText("Confirm folder", language)
                      : runtimeUiText("Choose folder", language)}
                  </strong>

                  <small>
                    {setupWorking
                      ? runtimeUiText("Installation path verified", language)
                      : runtimeUiText("Verify the installation path", language)}
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
                    {uiText("SPLIT integration", language)}
                  </strong>

                  <small>
                    {setupWorking
                      ? runtimeUiText("Configuring Deadlock…", language)
                      : runtimeUiText("Configure Deadlock automatically", language)}
                  </small>
                </div>
              </div>

              <div className="setup-progress-info">
                <span aria-hidden="true">
                  i
                </span>

                <p>
                  {runtimeUiText("SPLIT will configure the required Deadlock integration automatically after confirmation.", language)}
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
  const healthProblems = [
    !status.integrationHealthy
      ? {
          tone: "error" as const,
          title: uiText("SPLIT integration", language),
          description: runtimeUiText("The Deadlock integration needs repair.", language),
        }
      : null,
    !status.hotkeysRunning
      ? {
          tone: "error" as const,
          title: uiText("Hotkey hook", language),
          description: runtimeUiText("The Windows keyboard hook is not running.", language),
        }
      : null,
    status.deadlockRunning &&
    !status.consoleWatcherRunning
      ? {
          tone: "error" as const,
          title: uiText("Console watcher", language),
          description: runtimeUiText("The Deadlock console watcher is not running.", language),
        }
      : null,
    status.presentationMaskActive
      ? {
          tone: "error" as const,
          title: uiText("Presentation mask", language),
          description: runtimeUiText("Deadlock presentation may still be paused.", language),
        }
      : null,
    status.cameraRuntimeChecked &&
    !status.cameraRuntimeReady
      ? {
          tone: "error" as const,
          title: uiText("Camera runtime", language),
          description: runtimeUiText("The tested camera runtime is unavailable.", language),
        }
      : null,
    !status.deadlockRunning
      ? {
          tone: "warning" as const,
          title: uiText("Deadlock process", language),
          description: runtimeUiText("Deadlock is not currently running.", language),
        }
      : null,
    status.deadlockRunning &&
    !status.consoleLogExists
      ? {
          tone: "warning" as const,
          title: uiText("Console log", language),
          description: runtimeUiText("Deadlock's console log is unavailable.", language),
        }
      : null,
  ].filter((problem) => problem !== null);

  const healthIssueCount = healthProblems.filter(
    (problem) => problem.tone === "error",
  ).length;
  const healthWarningCount = healthProblems.filter(
    (problem) => problem.tone === "warning",
  ).length;
  const healthProblemCount = healthProblems.length;

  const healthTone: StatusTone =
    healthIssueCount > 0
      ? "error"
      : healthWarningCount > 0
        ? "warning"
        : "ok";

  const healthHeadline = runtimeUiText(healthProblemCount > 0 ? "Operational with issues" : "Operational", language);
  const healthDescription = healthProblemCount > 0
    ? `${healthProblemCount} ${runtimeUiText(healthProblemCount === 1 ? "issue detected" : "issues detected", language)}.`
    : runtimeUiText("No issues detected.", language);

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
            aria-label={`${runtimeUiText("Screenshot for", language)} ${screenshotViewer.name}`}
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
                  {uiText("Saved screenshot", language)}
                </span>
              </div>

              <button
                type="button"
                aria-label={uiText("Close screenshot", getLanguage())}
                title={uiText("Close", getLanguage())}
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
                alt={`${runtimeUiText("Screenshot for", language)} ${screenshotViewer.name}`}
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
              {uiText("Right click a saved slot to open its screenshot · Esc to close", language)}
            </footer>
          </section>
        </div>
      )}

      {folderReminder?.configuredPath && (
        <div className="confirmation-backdrop">
          <section
            className="confirmation-dialog folder-reminder-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="folder-reminder-title"
          >
            <h3 id="folder-reminder-title" className="confirmation-title">
              {uiText("It's been a while since you last opened SPLIT.", language)}
            </h3>
            <p className="confirmation-message">
              {uiText("Is this still your Deadlock folder?", language)}
            </p>
            <code className="folder-reminder-path">
              {folderReminder.configuredPath}
            </code>
            <div className="confirmation-actions">
              <button
                className="secondary-button"
                type="button"
                disabled={setupWorking}
                onClick={() => void chooseReminderFolder()}
              >
                {uiText("Choose another folder", language)}
              </button>
              <button
                className="primary-button"
                type="button"
                disabled={setupWorking}
                onClick={() => setFolderReminder(null)}
              >
                {uiText("Yes, keep this folder", language)}
              </button>
            </div>
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
                  {uiText("This shortcut may interfere with gameplay", language)}
                </h3>

                <p className="confirmation-description">
                  {runtimeUiText("SPLIT captures this key combination before Deadlock receives it.", language)}
                </p>

                <div className="gameplay-hotkey-selected">
                  <span>{uiText("Selected shortcut", language)}</span>

                  <strong>
                    {formatHotkey(
                      pendingGameplayHotkey.hotkey,
                    )}
                  </strong>
                </div>

                <p className="gameplay-hotkey-note">
                  {runtimeUiText("If these keys are also used in-game, those actions may not work while the shortcut is being triggered.", language)}
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
                {uiText("Don't remind me again", language)}
              </span>
            </label>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                onClick={cancelGameplayHotkey}
              >
                {uiText("Cancel", language)}
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={() =>
                  void confirmGameplayHotkey()
                }
              >
                {uiText("Use anyway", language)}
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
                  {uiText("Enter Favorite Mode?", language)}
                </h3>

                <p className="confirmation-description">
                  {runtimeUiText("Favorite Mode uses one shared bank of 8 Favorites instead of presets.", language)}
                </p>

                <div className="favorite-mode-summary">
                  <div>
                    <span>{uiText("Presets", language)}</span>
                    <strong>{uiText("Disabled", language)}</strong>
                  </div>

                  <div>
                    <span>{uiText("Favorites", language)}</span>
                    <strong>{runtimeUiText("8 shared slots", language)}</strong>
                  </div>
                </div>

                <p className="favorite-mode-note">
                  {runtimeUiText("Changes made here affect these same Favorites every time you return.", language)}
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
                {uiText("Don't remind me again", language)}
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
                {uiText("Leave Favorites", language)}
              </button>

              <button
                className="preset-button confirmation-primary-button"
                type="button"
                onClick={continueFavoriteMode}
              >
                {uiText("Continue", language)}
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
                  {runtimeUiText("Choose a Favorite slot for", language)}{" "}
                  <strong>
                    &quot;
                    {pendingFavoriteCopy.sourceName}
                    &quot;
                  </strong>
                </h3>

                <p className="confirmation-description">
                  {runtimeUiText("The complete save will be copied, including position, camera, name, timestamp and color.", language)}
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
                        {runtimeUiText("Favorite", language)} {favorite.slot}
                      </span>

                      <span
                        className={`favorite-destination-state ${
                          favorite.occupied
                            ? "occupied"
                            : "empty"
                        }`}
                      >
                        {favorite.occupied
                          ? runtimeUiText("Occupied", language) : runtimeUiText("Empty", language)}
                      </span>
                    </div>

                    <strong className="favorite-destination-name">
                      {favorite.occupied
                        ? favorite.name
                        : runtimeUiText("Available slot", language)}
                    </strong>

                    <span className="favorite-destination-hint">
                      {favorite.occupied
                        ? runtimeUiText("Overwrite this favorite", language) : runtimeUiText("Save here", language)}
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
                {uiText("Cancel", language)}
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
                  {runtimeUiText("Replace Favorite", language)}{" "}
                  {pendingFavoriteOverwrite.target.slot}?
                </h3>

                <p className="confirmation-description">
                  {runtimeUiText("This slot already contains a Favorite. Choose whether to replace it with the selected savestate.", language)}
                </p>

                <div className="favorite-overwrite-comparison">
                  <div>
                    <span>{uiText("Current", language)}</span>

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
                    <span>{uiText("Replace with", language)}</span>

                    <strong>
                      {pendingFavoriteOverwrite.sourceName}
                    </strong>
                  </div>
                </div>

                <p className="favorite-overwrite-undo">
                  {uiText("This change can be undone afterwards.", language)}
                </p>
              </div>
            </div>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                disabled={favoriteCopyWorking}
                onClick={cancelFavoriteOverwrite}
              >{runtimeUiText("Back", language)}</button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                disabled={favoriteCopyWorking}
                onClick={() =>
                  void confirmFavoriteOverwrite()
                }
              >
                {favoriteCopyWorking
                  ? runtimeUiText("Replacing…", language) : runtimeUiText("Replace Favorite", language)}
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
                  {uiText("Import preset?", language)}
                </h3>

                <p className="confirmation-description">
                  {runtimeUiText("The current preset will be replaced by the imported preset.", language)}
                </p>

                <div className="preset-import-comparison">
                  <div>
                    <span>{uiText("Current", language)}</span>

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
                    <span>{uiText("Import", language)}</span>

                    <strong>
                      {pendingImportPreset.importedName}
                    </strong>
                  </div>
                </div>

                <p className="preset-import-undo">
                  {uiText("This change can be undone afterwards.", language)}
                </p>
              </div>
            </div>

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                onClick={cancelPresetImport}
              >
                {uiText("Cancel", language)}
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={() =>
                  void confirmPresetImport()
                }
              >
                {uiText("Import preset", language)}
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
                  {uiText("Clear preset?", language)}
                </h3>

                <p className="confirmation-description">
                  {runtimeUiText("All savestates in this preset will be cleared.", language)}
                </p>

                <div className="clear-preset-target">
                  <span>{uiText("Preset", language)}</span>

                  <strong>
                    {pendingClearPreset.name}
                  </strong>

                  <small>
                    {runtimeUiText("8 savestate slots", language)}
                  </small>
                </div>

                <p className="clear-preset-undo">
                  {uiText("This change can be undone afterwards.", language)}
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
                {uiText("Don't ask me again", language)}
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
                {uiText("Cancel", language)}
              </button>

              <button
                className="preset-button preset-clear-button"
                type="button"
                onClick={() =>
                  void confirmClearPreset()
                }
              >
                {uiText("Clear preset", language)}
              </button>
            </div>
          </section>
        </div>
      )}

      {pendingPresetRename && (
        <div
          className="confirmation-backdrop"
          onMouseDown={(event) => {
            if (
              event.target === event.currentTarget &&
              !renamingPreset
            ) {
              setPendingPresetRename(null);
            }
          }}
        >
          <form
            className="confirmation-dialog preset-rename-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rename-preset-title"
            onSubmit={(event) => {
              event.preventDefault();
              void submitPresetRename();
            }}
            onKeyDown={(event) => {
              if (
                event.key === "Escape" &&
                !renamingPreset
              ) {
                event.preventDefault();
                setPendingPresetRename(null);
              }
            }}
          >
            <h3
              id="rename-preset-title"
              className="confirmation-title"
            >
              {uiText("Rename preset", language)}
            </h3>

            <p className="confirmation-description">
              {uiText("Preset", language)} {pendingPresetRename.preset}
            </p>

            <label className="preset-rename-field">
              <span>{uiText("Preset name", language)}</span>
              <input
                ref={presetRenameInputRef}
                type="text"
                value={pendingPresetRename.value}
                disabled={renamingPreset}
                onChange={(event) =>
                  setPendingPresetRename((current) =>
                    current
                      ? {
                          ...current,
                          value: event.target.value,
                          error: null,
                        }
                      : current,
                  )
                }
              />
            </label>

            {pendingPresetRename.error && (
              <p className="preset-rename-error" role="alert">
                {pendingPresetRename.error}
              </p>
            )}

            <div className="confirmation-actions">
              <button
                className="preset-button"
                type="button"
                disabled={renamingPreset}
                onClick={() => setPendingPresetRename(null)}
              >
                {uiText("Cancel", language)}
              </button>

              <button
                className="preset-button confirmation-primary-button"
                type="submit"
                disabled={renamingPreset}
              >
                {renamingPreset ? runtimeUiText("Renaming…", language) : uiText("Rename", language)}
              </button>
            </div>
          </form>
        </div>
      )}

      <aside className="sidebar">
        <div className="brand" aria-label="SPLIT">
          <strong>SPLIT</strong>
        </div>

        <nav className="sidebar-nav" aria-label={uiText("Main navigation", getLanguage())}>
          <button
            type="button"
            className={activeView === "slots" && !favoriteMode ? "active" : ""}
            disabled={status.matchSafetyLocked && favoriteMode}
            onClick={() => {
              setActiveView("slots");
              if (favoriteMode) void leaveFavoriteMode();
            }}
          >
            <span>{uiText("Presets", language)}</span>
          </button>
          <button
            type="button"
            className={activeView === "slots" && favoriteMode ? "active" : ""}
            disabled={
              status.matchSafetyLocked ||
              savingSlot !== null ||
              loadingSlot !== null
            }
            onClick={() => {
              setActiveView("slots");
              if (!favoriteMode) void toggleFavorites();
            }}
          >
            <span>{uiText("Favorites", language)}</span>
            <kbd>
              {formatHotkey(
                hotkeySettings.favoriteMode,
              )}
            </kbd>
          </button>
          <button
            type="button"
            className={activeView === "movement" ? "active" : ""}
            onClick={() => setActiveView("movement")}
          >
            <span>{runtimeUiText("Movement Lab", language)}</span>
          </button>
          <button
            type="button"
            className={activeView === "settings" ? "active" : ""}
            onClick={() => setActiveView("settings")}
          >
            <span>{t("settings", language)}</span>
          </button>
        </nav>

        <div className="sidebar-status">
          <div>
            <StatusDot
              tone={status.deadlockRunning ? "ok" : "off"}
            />
            <strong>Deadlock</strong>
          </div>
          <span>{status.deadlockRunning ? t("connected", language) : t("notConnected", language)}</span>
        </div>
      </aside>

      <main className="workspace">
      <header className={`topbar ${activeView === "slots" ? "slots-topbar" : ""}`}>
        <div>
          <h1>
            {activeView === "settings"
              ? t("settings", language)
              : activeView === "movement"
                ? runtimeUiText("Movement Lab", language)
              : favoriteMode
                ? t("favorites", language)
                : t("savestates", language)}
          </h1>

          {(activeView === "settings" || !favoriteMode) && (
            <p className="subtitle">
              {activeView === "settings"
                ? t("preferences", language)
                : activeView === "movement"
                  ? runtimeUiText("Record routes, race your ghost and track your PB.", language)
                : presetNames[activePreset - 1] ?? `${runtimeUiText("Preset", language)} ${activePreset}`}
            </p>
          )}
        </div>

        {activeView === "slots" && !favoriteMode ? (
          <div className="slots-v1-ticker" aria-live="off">
            <span className="slots-v1-rail" aria-hidden="true" />
            <span className={`slots-v1-message ${v1PhrasePhase}`}>
              {V1_HEADER_PHRASES[v1PhraseIndex]}
            </span>
            <span className="slots-v1-rail" aria-hidden="true" />
          </div>
        ) : null}

        {activeView === "slots" ? (
          <div className="history-actions header-history-actions">
            <button
              className="preset-button"
              type="button"
              disabled={
                status.matchSafetyLocked ||
                !historyState.canUndo ||
                savingSlot !== null ||
                loadingSlot !== null
              }
              onClick={() => void runHistoryAction("undo_last_action")}
            >
              {t("undo", language)}{" "}
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
                status.matchSafetyLocked ||
                !historyState.canRedo ||
                savingSlot !== null ||
                loadingSlot !== null
              }
              onClick={() => void runHistoryAction("redo_last_action")}
            >
              {t("redo", language)}{" "}
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
                ? t("checking", language)
                : t("refresh", language)}
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
                  ? uiText("Copying…", language)
                  : diagnosticCopied
                    ? uiText("Copied!", language)
                    : uiText("Copy diagnostics", language)}
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
                  ? t("checking", language)
                  : t("refresh", language)}
              </button>
            </div>
          )
        )}
      </header>

      {activeView === "settings" && (
        <nav
          className="settings-subnav"
          aria-label={uiText("Settings sections", language)}
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
            {t("general", language)}
          </button>

          <button
            type="button"
            className={
              activeSettingsSection === "discord"
                ? "active"
                : ""
            }
            onClick={() =>
              setActiveSettingsSection(
                "discord",
              )
            }
          >
            {uiText("Discord Rich Presence", language)}
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
            {uiText("Hotkeys", language)}
          </button>

          {isBorderlessEdition && (
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
              {uiText("In-Game Notifications", language)}
            </button>
          )}

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
            {uiText("Diagnostics", language)}
          </button>

          <button
            type="button"
            className={
              activeSettingsSection === "about"
                ? "active"
                : ""
            }
            onClick={() =>
              setActiveSettingsSection("about")
            }
          >
            {uiText("About", language)}
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
              {t("systemHealth", language).toUpperCase()}
            </span>

            <strong>
              {healthHeadline}
            </strong>

            <p className="health-summary-description">
              {healthDescription}
            </p>
          </div>
        </div>

        {healthProblemCount > 0 && (
          <div className="health-summary-counts">
            <button
              className={`health-summary-toggle ${healthTone}`}
              type="button"
              aria-expanded={diagnosticSummaryExpanded}
              aria-controls="diagnostic-summary-problems"
              onClick={() =>
                setDiagnosticSummaryExpanded((expanded) => !expanded)
              }
            >
              <span>
                {healthProblemCount} {uiText(healthProblemCount === 1 ? "issue detected" : "issues detected", language)}
              </span>
              <span className="health-summary-chevron" aria-hidden="true">
                ⌄
              </span>
            </button>
          </div>
        )}

        {healthProblemCount > 0 && diagnosticSummaryExpanded && (
          <div
            className="health-summary-problems"
            id="diagnostic-summary-problems"
          >
            <ul>
              {healthProblems.map((problem) => (
                <li key={problem.title}>
                  <StatusDot tone={problem.tone} />
                  <div>
                    <strong>{uiText(problem.title, language)}</strong>
                    <span>{problem.description}</span>
                  </div>
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>
      )}


      {activeView === "movement" && (
        <section className="movement-lab">
          <div
            className={`movement-session-banner ${movementStatus?.recording ? "is-recording" : movementStatus?.racePhase === "running" ? "is-running" : movementStatus?.racePhase === "finished" && movementStatus.lastWasPb ? "is-pb" : ""}`}
            role="status"
            aria-live="polite"
          >
            <span className="movement-session-indicator" aria-hidden="true" />
            <div>
              <strong>
                {!movementStatus ? runtimeUiText("Movement Lab offline", language)
                  : movementStatus.recording ? runtimeUiText("Recording route", language)
                  : movementStatus.racePhase === "armed" ? runtimeUiText("Ready to race", language)
                  : movementStatus.racePhase === "running" ? runtimeUiText("Race in progress", language)
                  : movementStatus.racePhase === "finished" && movementStatus.lastWasPb ? runtimeUiText("New personal best!", language)
                  : movementStatus.racePhase === "finished" ? runtimeUiText("Run finished", language)
                  : movementStatus.hasRoute ? runtimeUiText("Route ready", language)
                  : runtimeUiText("Record your first route", language)}
              </strong>
              <span>
                {!movementStatus ? runtimeUiText("Start the Movement Runtime to begin.", language)
                  : movementStatus.recording ? `${movementStatus.ticks.toLocaleString()} ${runtimeUiText("ticks captured", language)}`
                  : movementStatus.racePhase === "armed" ? runtimeUiText("Enter the start zone to begin timing.", language)
                  : movementStatus.racePhase === "running" ? `${movementStatus.elapsedSeconds.toFixed(3)}s · ${movementStatus.nextCheckpoint}/${movementStatus.checkpointCount} ${runtimeUiText("checkpoints", language)}`
                  : movementStatus.racePhase === "finished" && movementStatus.lastSeconds != null
                    ? `${movementStatus.lastSeconds.toFixed(3)}s${movementStatus.lastDeltaSeconds != null ? ` · ${movementStatus.lastDeltaSeconds > 0 ? "+" : ""}${movementStatus.lastDeltaSeconds.toFixed(3)}s ${runtimeUiText("vs previous PB", language)}` : ""}`
                  : movementStatus.hasRoute ? runtimeUiText("Use Race or Replay, or record a new route.", language)
                  : runtimeUiText("Use the Record shortcut while playing Deadlock.", language)}
              </span>
            </div>
          </div>
          <div className="movement-runtime-card">
            <div>
              <span className="label">{runtimeUiText("MOVEMENT RUNTIME", language)}</span>
              <h2>{movementStatus ? runtimeUiText("Runtime connected", language) : runtimeUiText("Runtime offline", language)}</h2>
              <p>{movementStatus ? (movementStatus.playerConnected ? runtimeUiText("Deadlock player connected to the local Movement Lab server.", language) : runtimeUiText("Runtime is ready. Connect Deadlock to the local server.", language)) : runtimeUiText("Start split-movement-runtime.exe in server mode to use Movement Lab.", language)}</p>
            </div>
            <div className="movement-runtime-badge"><StatusDot tone={movementStatus ? "ok" : "off"} /><span>{movementStatus ? `v${movementStatus.version}` : runtimeUiText("OFFLINE", language)}</span></div>
          </div>

          <div className="movement-display-card">
            <div className="movement-display-heading">
              <div>
                <p className="label">{runtimeUiText("IN-GAME CONTROL", language)}</p>
                <h2>{runtimeUiText("Panorama controls", language)}</h2>
              </div>
              <span>{movementStatus?.playerConnected ? runtimeUiText("Available in Deadlock", language) : runtimeUiText("Waiting for player", language)}</span>
            </div>
            <p className="movement-display-note">{runtimeUiText("Record, retry, replay, race and practice assists are controlled from the Movement Lab Panorama inside Deadlock.", language)}</p>
          </div>

          <div className="movement-display-card">
            <div className="movement-display-heading">
              <div><p className="label">{runtimeUiText("DISPLAY PREFERENCES", language)}</p><h2>{runtimeUiText("Movement Lab visuals", language)}</h2></div>
              <span>{movementDisplaySaving ? runtimeUiText("Syncing…", language) : movementStatus ? runtimeUiText("Runtime connected", language) : runtimeUiText("Saved locally", language)}</span>
            </div>
            <fieldset className="movement-visual-modes">
              <legend>{runtimeUiText("Route visualization", language)}</legend>
              {([
                ["both", "Ghost + tracer"],
                ["ghost", "3D ghost"],
                ["tracer", "Light trail"],
                ["off", "Off"],
              ] as const).map(([mode, label]) => (
                <label key={mode} className={movementDisplay.visual === mode ? "selected" : ""}>
                  <input type="radio" name="movement-visual-mode" checked={movementDisplay.visual === mode} onChange={() => updateMovementDisplay({ visual: mode })} />
                  <span>{runtimeUiText(label, language)}</span>
                </label>
              ))}
            </fieldset>
            <div className="movement-display-toggles">
              {([
                ["rings", "Start and finish rings"],
                ["checkpoints", "Checkpoint markers"],
                ["hud", "In-game timing HUD"],
              ] as const).map(([key, label]) => (
                <label key={key}>
                  <input type="checkbox" checked={movementDisplay[key]} onChange={(event) => updateMovementDisplay({ [key]: event.target.checked })} />
                  <span>{runtimeUiText(label, language)}</span>
                </label>
              ))}
            </div>
            <p className="movement-display-note">{runtimeUiText("Preferences are saved and sent to the runtime. Native in-game trail, rings and HUD rendering are not yet available; these controls prepare their configuration.", language)}</p>
          </div>

          <div className="movement-stats-grid">
            <article><span>{runtimeUiText("Route", language)}</span><strong>{movementStatus?.hasRoute ? `${movementStatus.ticks.toLocaleString()} ticks` : "—"}</strong></article>
            <article><span>{runtimeUiText("Race status", language)}</span><strong>{movementStatus?.recording ? runtimeUiText("Recording", language) : movementStatus?.racePhase ? runtimeUiText(({ idle: "Idle", armed: "Armed", running: "Running", finished: "Finished" } as const)[movementStatus.racePhase], language) : "—"}</strong></article>
            <article><span>{runtimeUiText("Current", language)}</span><strong className="movement-timer-value">{movementStatus && movementStatus.racePhase === "running" ? `${movementStatus.elapsedSeconds.toFixed(3)}s` : "—"}</strong></article>
            <article><span>{runtimeUiText("Personal best", language)}</span><strong>{movementStatus?.personalBestSeconds != null ? `${movementStatus.personalBestSeconds.toFixed(3)}s` : "—"}</strong></article>
            <article><span>{runtimeUiText("Last run", language)}</span><strong>{movementStatus?.lastSeconds != null ? `${movementStatus.lastSeconds.toFixed(3)}s` : "—"}</strong></article>
            <article><span>{runtimeUiText("Checkpoint", language)}</span><strong>{movementStatus ? `${movementStatus.nextCheckpoint}/${movementStatus.checkpointCount}` : "—"}</strong></article>
          </div>

          {movementStatus && movementStatus.checkpointCount > 0 && (
            <div className="movement-splits-card">
              <div className="savestates-header"><div><p className="label">{runtimeUiText("SPLITS", language)}</p><h2>{runtimeUiText("Checkpoint times", language)}</h2></div></div>
              <div className="movement-split-list">
                {Array.from({ length: movementStatus.checkpointCount }, (_, index) => {
                  const current = movementStatus.currentSplits[index];
                  const pb = movementStatus.personalBestSplits[index];
                  const finished = movementStatus.racePhase === "finished";
                  const delta = finished
                    ? movementStatus.lastSplitDeltas?.[index]
                    : current != null && pb != null ? current - pb : undefined;
                  return (
                    <div key={index} className={delta == null ? "" : delta < -0.0005 ? "movement-split-faster" : delta > 0.0005 ? "movement-split-slower" : "movement-split-even"}>
                      <span>{index + 1}</span>
                      <strong>{current != null ? `${current.toFixed(3)}s` : "—"}</strong>
                      <small>{delta != null ? `${delta > 0 ? "+" : ""}${delta.toFixed(3)}s` : pb != null ? `PB ${pb.toFixed(3)}s` : "—"}</small>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
          {movementError && <div className="error-box movement-error">{movementError}</div>}
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
                  {uiText("Deadlock is not running", language)}
                </strong>

                <span>
                  {uiText("SPLIT is ready. Hotkeys and teleporting will become available when Deadlock starts.", language)}
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
                ? uiText("Launching…", language)
                : uiText("Launch", language)}
            </button>
          </div>
        )}

      {activeView === "slots" && (
      <>
      {status.matchSafetyLocked && (
        <div className="match-safety-banner" role="status">
          <strong>{uiText("MATCH IN PROGRESS", language)}</strong>
          <span>{uiText("SPLIT is locked until you return to the Hideout.", language)}</span>
        </div>
      )}
    <fieldset
      className={`savestates-section ${
        status.matchSafetyLocked ? "match-safety-locked" : ""
      }`}
      disabled={status.matchSafetyLocked}
      aria-disabled={status.matchSafetyLocked}
    >
      <div className="savestates-header">
        <div>
          <p className="label">
            {uiText("SAVESTATES", language)}
          </p>

          <h2>
            {uiText("Position slots", language)}
          </h2>
        </div>

        <span className="savestates-hint">
          {uiText("Save Alt+F1–F8 · Load F1–F8", language)}
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
                        `${uiText("Preset", language)} ${preset}`}
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
                          aria-label={uiText("Preset actions", getLanguage())}
                          title={uiText("Preset actions", getLanguage())}
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
                              ? runtimeUiText("Renaming…", language) : uiText("Rename preset", language)}
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
                              ? runtimeUiText("Exporting…", language) : runtimeUiText("Export preset", language)}
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
                              ? runtimeUiText("Importing…", language) : uiText("Import preset", language)}
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
                              ? runtimeUiText("Clearing…", language) : uiText("Clear preset", language)}
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
        {uiText("Favorites", language)} ·{" "}
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
          {runtimeUiText("Undo", language)}&nbsp;&nbsp;
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
          {runtimeUiText("Redo", language)}&nbsp;&nbsp;
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
                  ? `${runtimeUiText("Favorite", language)} ${slot}` : `${runtimeUiText("Slot", language)} ${slot}`
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
                    language,
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
                  title={position && metadata?.screenshot ? runtimeUiText("Right click to view screenshot", language)
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
                            {uiText("Position captured", language)}
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
                          ? runtimeUiText("Loading…", language)
                          : runtimeUiText("Load", language)}
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
                        {uiText("Empty slot", language)}
                      </span>

                      <small>
                        {uiText("Save current position", language)}
                      </small>
                    </button>
                  )}
                </div>

                <details className="slot-card-menu">
                  <summary
                    aria-label={`${runtimeUiText("Open actions for", language)} ${displayName}`}
                    title={uiText("Slot actions", getLanguage())}
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
                          {uiText("Copy position", language)}
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
                          {runtimeUiText("Load", language)}
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
                          {uiText("View screenshot", language)}

                          <span>
                            {uiText("Right click", language)}
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
                        ? uiText("Overwrite", language)
                        : runtimeUiText("Save", language)}
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

                        beginSlotRename(
                          slot,
                          displayName,
                        );
                      }}
                    >
                      {uiText("Rename", language)}
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
                        {uiText("Save to Favorite", language)}
                      </button>
                    )}

                    {position && (
                      <>
                        <div className="slot-menu-divider" />

                        <span className="slot-menu-label">
                          {uiText("Color", language)}
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
                                  aria-label={`${label} · ${runtimeUiText("Slot color", language)}`}
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
                      {uiText("Clear", language)}
                    </button>
                  </div>
                </details>

                <div className="slot-card-info">
                  <div className="slot-card-copy">
                    {editingSlot === slot ? (
                      <input
                        ref={slotRenameInputRef}
                        className="slot-card-name-input"
                        value={editingSlotName}
                        aria-label={`${runtimeUiText("Rename slot", language)} ${slot}`}
                        onChange={(event) =>
                          setEditingSlotName(
                            event.target.value,
                          )
                        }
                        onPointerDown={(event) =>
                          event.stopPropagation()
                        }
                        onClick={(event) =>
                          event.stopPropagation()
                        }
                        onDoubleClick={(event) =>
                          event.stopPropagation()
                        }
                        onKeyDown={(event) => {
                          event.stopPropagation();

                          if (event.key === "Enter") {
                            event.preventDefault();
                            void finishSlotRename(
                              slot,
                              displayName,
                              true,
                            );
                          } else if (
                            event.key === "Escape"
                          ) {
                            event.preventDefault();
                            void finishSlotRename(
                              slot,
                              displayName,
                              false,
                            );
                          }
                        }}
                        onBlur={() =>
                          void finishSlotRename(
                            slot,
                            displayName,
                            true,
                          )
                        }
                      />
                    ) : (
                      <button
                        className="slot-card-name-button"
                        type="button"
                        title={displayName}
                        disabled={
                          savingSlot !== null ||
                          loadingSlot !== null
                        }
                        onClick={(event) => {
                          event.stopPropagation();
                          beginSlotRename(
                            slot,
                            displayName,
                          );
                        }}
                      >
                        {displayName}
                      </button>
                    )}

                    <span>
                      {savedAge ??
                        (position
                          ? runtimeUiText("Saved", language)
                          : runtimeUiText("No position saved", language))}
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
    </fieldset>
      </>
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
                  {language === "en" ? "General settings" : t("general", language)}
                </h2>

                <p>
                  {uiText("Configure desktop behavior, Quick Access, appearance and warnings.", language)}
                </p>
              </div>
            </div>


          <div className="general-settings-group">
            <h3>{t("language", language).toUpperCase()}</h3>
            <div className="general-setting-row">
              <div>
                <strong>{t("language", language)}</strong>
                <span>{t("chooseLanguage", language)}</span>
              </div>
              <select className="language-setting-select" aria-label={t("language", language)} value={automaticLanguage ? "auto" : language} onChange={(event) => changeLanguage(event.target.value)}>
                <option value="auto">{t("auto", language)}</option>
                {languages.map((code) => <option key={code} value={code}>{languageNames[code]}</option>)}
              </select>
            </div>
          </div>

          <div className="general-settings-group">
                <h3>{uiText("APPEARANCE", language)}</h3>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Slot color style", language)}
                  </strong>

                  <span>
                    {uiText("Choose how assigned colors are displayed on savestate cards.", language)}
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
                    {uiText("Tint", language)}
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
                    {uiText("Dot", language)}
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
                    {uiText("Accent", language)}
                  </button>
                </div>
              </div>  


            <div className="general-settings-list">
              <div className="general-settings-group">
                <h3>{uiText("STARTUP", language)}</h3>

                <div className="general-setting-row">
                  <div>
                    <strong>{uiText("Startup sound", language)}</strong>
                    <span>
                      {uiText("Play the SPLIT startup sound when the app launches.", language)}
                    </span>
                  </div>

                  <button
                    className={`general-toggle ${
                      startupSoundSettings.enabled ? "active" : ""
                    }`}
                    type="button"
                    role="switch"
                    aria-checked={startupSoundSettings.enabled}
                    onClick={toggleStartupSound}
                  >
                    {startupSoundSettings.enabled ? uiText("On", language) : uiText("Off", language)}
                  </button>
                </div>

                <div className="general-setting-row startup-sound-volume-row">
                  <div>
                    <strong id="startup-sound-volume-label">{uiText("Volume", language)}</strong>
                    <span>{uiText("Adjust playback without changing Windows volume.", language)}</span>
                  </div>

                  <div className="startup-sound-volume-control">
                    <input
                      type="range"
                      min="0"
                      max="100"
                      step="1"
                      value={startupSoundSettings.volume}
                      aria-labelledby="startup-sound-volume-label"
                      aria-valuetext={`${startupSoundSettings.volume}%`}
                      onChange={(event) => {
                        const next = {
                          ...startupSoundSettings,
                          volume: Number(event.currentTarget.value),
                        };
                        setStartupSoundSettings(next);
                        scheduleStartupSoundVolumeSave(next);
                      }}
                      onPointerUp={(event) =>
                        flushStartupSoundVolumeSave({
                          ...startupSoundSettings,
                          volume: Number(event.currentTarget.value),
                        })
                      }
                      onKeyUp={(event) =>
                        flushStartupSoundVolumeSave({
                          ...startupSoundSettings,
                          volume: Number(event.currentTarget.value),
                        })
                      }
                      onBlur={(event) =>
                        flushStartupSoundVolumeSave({
                          ...startupSoundSettings,
                          volume: Number(event.currentTarget.value),
                        })
                      }
                    />
                    <output aria-live="polite">
                      {startupSoundSettings.volume}%
                    </output>
                    <button
                      className="general-restore-button startup-sound-preview"
                      type="button"
                      onClick={() => void previewStartupSound()}
                    >
                      {uiText("Preview", language)}
                    </button>
                  </div>
                </div>

                {(startupSoundSaving || startupSoundFeedback) && (
                  <p
                    className={`startup-sound-feedback ${
                      startupSoundFeedback ? "error" : ""
                    }`}
                    role={startupSoundFeedback ? "alert" : "status"}
                  >
                    {startupSoundFeedback ?? runtimeUiText("Saving startup sound settings…", language)}
                  </p>
                )}

                <div className="general-setting-row">
                  <div>
                    <strong>{uiText("Focus Deadlock when SPLIT starts", language)}</strong>
                    <span>
                      {uiText("Brings the Deadlock window to the foreground when SPLIT starts.", language)}
                    </span>
                  </div>

                  <button
                    className={`general-toggle ${
                      focusDeadlockOnStartup ? "active" : ""
                    }`}
                    type="button"
                    role="switch"
                    aria-checked={focusDeadlockOnStartup}
                    disabled={
                      focusDeadlockOnStartupLoading || focusDeadlockOnStartupSaving
                    }
                    onClick={() => void toggleFocusDeadlockOnStartup()}
                  >
                    {focusDeadlockOnStartupSaving
                      ? "Saving…"
                      : focusDeadlockOnStartupLoading
                        ? "Checking…"
                        : focusDeadlockOnStartup
                          ? uiText("On", language)
                          : uiText("Off", language)}
                  </button>
                </div>
              </div>

              <div className="general-settings-group">
                <h3>{uiText("DESKTOP", language)}</h3>

              <div className="general-setting-row">
                <div>
                  <strong>{uiText("Discord Rich Presence", language)}</strong>
                  <span>
                    {uiText("Share your current Deadlock activity on Discord.", language)}
                  </span>
                </div>

                <button
                  className="general-restore-button"
                  type="button"
                  onClick={() => setActiveSettingsSection("discord")}
                >
                  {uiText("Configure", language)}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Launch with Windows", language)}
                  </strong>

                  <span>
                    {runtimeUiText("Start SPLIT automatically when you sign in to Windows.", language)}
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
                    ? runtimeUiText("Saving…", language)
                    : autostartLoading
                      ? runtimeUiText("Checking…", language)
                      : autostartEnabled
                        ? uiText("On", language)
                        : uiText("Off", language)}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Start minimized to tray", language)}
                  </strong>

                  <span>
                    {autostartEnabled
                      ? runtimeUiText("Keep SPLIT hidden in the system tray when it starts with Windows.", language)
                      : runtimeUiText("Enable Launch with Windows to use this option.", language)}
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
                    ? runtimeUiText("Saving…", language)
                    : startMinimizedLoading
                      ? runtimeUiText("Checking…", language)
                      : startMinimized
                        ? uiText("On", language)
                        : uiText("Off", language)}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Close button behavior", language)}
                  </strong>

                  <span>
                    {runtimeUiText("Choose what happens when you close the SPLIT window.", language)}
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
                    {uiText("Minimize to tray", language)}
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
                    {uiText("Quit SPLIT", language)}
                  </button>
                </div>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Window size & position", language)}
                  </strong>

                  <span>
                    {runtimeUiText("Restore the default window size and center SPLIT on the screen.", language)}
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
                    ? runtimeUiText("Resetting…", language)
                    : runtimeUiText("Reset", language)}
                </button>
              </div>

              </div>

              {isBorderlessEdition && (
              <div
                className={`general-settings-group quick-access-settings-group ${
                  quickAccessSettings.enabled
                    ? ""
                    : "disabled"
                }`}
              >
                <h3>{uiText("QUICK ACCESS", language)}</h3>

                <div className="general-setting-row">
                  <div>
                    <strong>{uiText("Enabled", language)}</strong>
                    <span>
                      {uiText("Show the Quick Access overlay in Deadlock.", language)}
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
                    {quickAccessSettings.enabled ? uiText("On", language) : uiText("Off", language)}
                  </button>
                </div>

                <div className="general-setting-row quick-access-dependent-setting">
                  <div>
                    <strong>{uiText("Shortcut", language)}</strong>
                    <span>{uiText("Hold shortcut to interact", language)}</span>
                  </div>

                  <div className="quick-access-shortcut-control">
                    <kbd>
                      {isHotkeyTarget(
                        capturingHotkey,
                        { group: "quickAccess" },
                      )
                        ? runtimeUiText("Press a shortcut...", language)
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
                        ? uiText("Cancel", language) : runtimeUiText("Change", language)}
                    </button>
                  </div>
                </div>

                <div className="general-setting-row quick-access-dependent-setting">
                  <div>
                    <strong>{uiText("Position", language)}</strong>
                    <span>
                      {uiText("Anchor the overlay inside the Deadlock window.", language)}
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
                          {runtimeUiText(position === "left" ? "Left" : "Right", language)}
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
              )}

              

              </div>

              <div className="general-settings-group">
                <h3>{runtimeUiText("CONFIRMATIONS & WARNINGS", language)}</h3>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Clear preset confirmation", language)}
                  </strong>

                  <span>
                    {runtimeUiText("Show the confirmation dialog before clearing an entire preset.", language)}
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
                    ? runtimeUiText("Restored", language)
                    : runtimeUiText("Restore confirmation", language)}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Gameplay hotkey warning", language)}
                  </strong>

                  <span>
                    {runtimeUiText("Warn when a shortcut may interfere with Deadlock controls.", language)}
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
                    ? runtimeUiText("Restored", language)
                    : runtimeUiText("Restore warning", language)}
                </button>
              </div>

              <div className="general-setting-row">
                <div>
                  <strong>
                    {uiText("Favorite Mode warning", language)}
                  </strong>

                  <span>
                    {runtimeUiText("Show the information dialog when entering Favorite Mode.", language)}
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
                    ? runtimeUiText("Restored", language)
                    : runtimeUiText("Restore warning", language)}
                </button>
              </div>
              </div>
            </div>
          </section>
        )}

    {activeSettingsSection === "discord" && (
      <DiscordPresenceSettings />
    )}

    {activeSettingsSection === "hotkeys" && (    
      <section className="hotkey-settings-section">
        <div className="hotkey-settings-heading">
          <div>
            <p className="label">{uiText("HOTKEYS", language)}</p>
            <h2>{uiText("Keyboard shortcuts", language)}</h2>
          </div>
          <button
            className="preset-button"
            type="button"
            disabled={hotkeySettingsSaving}
            onClick={() => void resetHotkeys()}
          >
            {hotkeysRestored ? runtimeUiText("Restored", language) : runtimeUiText("Reset to defaults", language)}
          </button>
        </div>

        <p className="hotkey-settings-note">
          {uiText("Customize the shortcuts used while Deadlock is focused.", language)}
        </p>

        <div className="hotkey-categories">
          {([
            ["LOAD", "loadSlots", hotkeySettings.loadSlots],
            ["SAVE", "saveSlots", hotkeySettings.saveSlots],
          ] as const).map(([label, group, bindings]) => (
            <div className="hotkey-category" key={group}>
              <h3>{runtimeUiText(label, language)}</h3>
              {bindings.map((hotkey, index) => {
                const target: HotkeyTarget = { group, index };
                const capturing = isHotkeyTarget(capturingHotkey, target);
                return (
                  <div className={`hotkey-row ${capturing ? "capturing" : ""}`} key={`${group}-${index}`}>
                    <span>{label === "LOAD" ? runtimeUiText("Load", language) : runtimeUiText("Save", language)} {runtimeUiText("Slot", language)} {index + 1}</span>
                    <HotkeyKeys
                      hotkey={hotkey}
                      capturing={capturing}
                    />
                    <button
                      type="button"
                      disabled={hotkeySettingsSaving}
                      onClick={() => {
                        setHotkeyMessage(null);
                        setCapturingHotkey(capturing ? null : target);
                      }}
                    >
                      {capturing ? uiText("Cancel", language) : runtimeUiText("Change", language)}
                    </button>
                  </div>
                );
              })}
            </div>
          ))}

          <div className="hotkey-category hotkey-actions-category">
            <h3>{uiText("ACTIONS", language)}</h3>
            {([
              ["Undo", "undo", hotkeySettings.undo],
              ["Redo", "redo", hotkeySettings.redo],
              ["Cycle Preset", "cyclePreset", hotkeySettings.cyclePreset],
              ["Favorite Mode", "favoriteMode", hotkeySettings.favoriteMode],
            ] as const).map(([label, group, hotkey]) => {
              const target: HotkeyTarget = { group };
              const capturing = isHotkeyTarget(capturingHotkey, target);
              return (
                <div className="hotkey-action-item" key={group}>
                  <div className={`hotkey-row ${capturing ? "capturing" : ""}`}>
                    <span>{runtimeUiText(label, language)}</span>
                    <HotkeyKeys
                      hotkey={hotkey}
                      capturing={capturing}
                    />
                    <button
                      type="button"
                      disabled={hotkeySettingsSaving}
                      onClick={() => {
                        setHotkeyMessage(null);
                        setCapturingHotkey(capturing ? null : target);
                      }}
                    >
                      {capturing ? uiText("Cancel", language) : runtimeUiText("Change", language)}
                    </button>
                  </div>
                  {group === "cyclePreset" && isSimpleTextHotkey(hotkey) && (
                    <p className="hotkey-text-key-note">
                      {runtimeUiText("Text keys may be captured while the Deadlock console is open. Use a function key or modified shortcut to avoid interfering with typing.", language)}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </div>

        {hotkeyMessage && <p className="hotkey-message" role="alert">{hotkeyMessage}</p>}
      </section>
    )}
    

    {isBorderlessEdition && activeSettingsSection === "notifications" && (
      <section className="notification-settings-section">
        <div className="notification-settings-heading">
          <div>
            <p className="label">{runtimeUiText("IN-GAME NOTIFICATIONS", language)}</p>

            <h2>
              {uiText("In-Game notifications", language)}
            </h2>

            <p>
              {runtimeUiText("Configure how SPLIT notifications appear while playing Deadlock.", language)}
            </p>
          </div>

          <button
            className="general-restore-button notification-test-button"
            type="button"
            disabled={
              notificationTesting ||
              notificationSettingsSaving
            }
            onClick={() => void testNotification()}
          >
            {notificationTesting
              ? runtimeUiText("Testing…", language)
              : uiText("Test notification", language)}
          </button>
        </div>

        {notificationTestFeedback && (
          <p
            className={`notification-test-feedback ${
              notificationTestFeedback.error
                ? "error"
                : ""
            }`}
            role={
              notificationTestFeedback.error
                ? "alert"
                : "status"
            }
          >
            {notificationTestFeedback.text}
          </p>
        )}

        <div className="notification-settings-grid">
          <label className="notification-setting-row">
            <span>{uiText("Enabled", language)}</span>
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
              {notificationSettings.enabled ? uiText("ON", language) : uiText("OFF", language)}
            </button>
          </label>

          <label className="notification-setting-row">
            <span>{uiText("Position", language)}</span>
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
              <option value="topLeft">{uiText("Top Left", language)}</option>
              <option value="topRight">{uiText("Top Right", language)}</option>
              <option value="bottomLeft">{uiText("Bottom Left", language)}</option>
              <option value="bottomRight">{uiText("Bottom Right", language)}</option>
            </select>
          </label>

          <label className="notification-setting-row">
            <span>{uiText("Duration", language)}</span>
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
              <option value={500}>0.5 s</option>
              <option value={1000}>1.0 s</option>
              <option value={1500}>1.5 s</option>
              <option value={2000}>2.0 s</option>
              <option value={3000}>3.0 s</option>
            </select>
          </label>

          <label className="notification-setting-row">
            <span>{uiText("Use slot color", language)}</span>
            <button
              className={`notification-toggle ${
                notificationSettings.useSlotColor ? "active" : ""
              }`}
              type="button"
              role="switch"
              aria-checked={notificationSettings.useSlotColor}
              disabled={notificationSettingsSaving}
              onClick={() =>
                void updateNotificationSettings({
                  ...notificationSettings,
                  useSlotColor: !notificationSettings.useSlotColor,
                })
              }
            >
              {notificationSettings.useSlotColor ? uiText("ON", language) : uiText("OFF", language)}
            </button>
          </label>
        </div>
      </section>
    )}  

    {activeSettingsSection === "diagnostics" && ( 
      <section
        className="diagnostics-groups"
        aria-label={uiText("Deadlock diagnostics", getLanguage())}
      >
        <section className="diagnostics-group">
          <h2 className="diagnostics-group-title">
            {uiText("Runtime", language)}
          </h2>

          <div className="diagnostics-grid diagnostics-runtime-grid">
        <article className="status-card">
          <div className="status-heading">
            <StatusDot ok />

            <span>
              {uiText("SPLIT edition", language)}
            </span>
            <DiagnosticInfo language={language} text={"Shows which SPLIT edition is running. Borderless supports Quick Access and in-game notifications; Fullscreen uses keyboard shortcuts."} fix={"If this is not the edition you expected, launch the intended SPLIT executable."} />
          </div>

          <strong>
            {splitEditionLabel}
          </strong>
        </article>

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
              {uiText("SPLIT integration", language)}
            </span>
            <DiagnosticInfo language={language} text={"Checks whether SPLIT's Deadlock integration is configured correctly."} fix={"Click Repair integration, then Refresh. If it still fails, check the Deadlock installation path."} />
          </div>

          <strong>
            {status.integrationHealthy
              ? runtimeUiText("Healthy", language)
              : runtimeUiText("Needs attention", language)}
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
                ? runtimeUiText("Repairing…", language)
                : runtimeUiText("Repair integration", language)}
            </button>
          )}
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={status.deadlockRunning}
            />

            <span>
              {uiText("Deadlock process", language)}
            </span>
            <DiagnosticInfo language={language} text={"Shows whether SPLIT detects Deadlock. Not running is normal when the game is closed."} fix={"If Deadlock is open but not detected, restart SPLIT, check the installation path, then Refresh."} />
          </div>

          <strong>
            {status.deadlockRunning
              ? runtimeUiText("Running", language)
              : runtimeUiText("Not running", language)}
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
              {uiText("Hotkey hook", language)}
            </span>
            <DiagnosticInfo language={language} text={"Listens for SPLIT keyboard shortcuts."} fix={"If shortcuts stop working, close duplicate SPLIT instances and restart SPLIT. If Deadlock runs as administrator, match its privileges. Additional troubleshooting appears when the hook is Down."} />
          </div>

          <strong>
            {status.hotkeysRunning
              ? runtimeUiText("Running", language)
              : runtimeUiText("Down", language)}
          </strong>

          {!status.hotkeysRunning && (
            <div className="diagnostic-details">
              <div className="diagnostic-reason">
                <span>
                  {runtimeUiText("REASON", language)}
                </span>

                <code>
                  {status.hotkeysError ??
                    runtimeUiText("The Windows keyboard hook is not running.", language)}
                </code>
              </div>

              <div className="diagnostic-fix">
                <span>
                  {runtimeUiText("HOW TO FIX", language)}
                </span>

                <ol>
                  <li>
                    {runtimeUiText("Close every other running instance of SPLIT.", language)}
                  </li>

                  <li>
                    {uiText("Completely restart SPLIT.", language)}
                  </li>

                  <li>
                    {runtimeUiText("If Deadlock is running as administrator, run SPLIT with the same privilege level.", language)}
                  </li>

                  <li>
                    {runtimeUiText("Temporarily disable software that intercepts global keyboard input and test again.", language)}
                  </li>

                  <li>
                    {runtimeUiText("If the issue persists, copy the diagnostic report before reporting the problem.", language)}
                  </li>
                </ol>
              </div>

              <p className="diagnostic-description">
                {runtimeUiText("Hotkeys cannot currently be restarted safely inside the same SPLIT process. Restarting SPLIT is required.", language)}
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
              {uiText("Console watcher", language)}
            </span>
            <DiagnosticInfo language={language} text={"Reads Deadlock's console log for activity and position responses."} fix={"If the watcher stops working, check the Deadlock installation path and restart the game. Then use Retry watcher, which appears when an issue is detected."} />
          </div>

          <strong>
            {status.consoleWatcherRunning
              ? runtimeUiText("Running", language)
              : runtimeUiText("Down", language)}
          </strong>

          {!status.consoleWatcherRunning && (
            <div className="diagnostic-details">
              <div className="diagnostic-reason">
                <span>
                  {runtimeUiText("REASON", language)}
                </span>

                <code>
                  {status.consoleWatcherError ??
                    runtimeUiText("The console watcher is not running.", language)}
                </code>
              </div>

              <div className="diagnostic-fix">
                <span>
                  {runtimeUiText("HOW TO FIX", language)}
                </span>

                <ol>
                  <li>
                    {runtimeUiText("Make sure SPLIT points to the correct Deadlock installation.", language)}
                  </li>

                  <li>
                    {runtimeUiText("Check that the game\\citadel folder still exists.", language)}
                  </li>

                  <li>
                    {runtimeUiText("If console.log is missing, launch Deadlock once.", language)}
                  </li>

                  <li>
                    {uiText("Click Retry watcher below.", language)}
                  </li>

                  <li>
                    {runtimeUiText("If it still fails, check Windows permissions or antivirus software blocking SPLIT.", language)}
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
                    ? runtimeUiText("Restarting…", language)
                    : runtimeUiText("Retry watcher", language)}
                </button>
              </div>
            </div>
          )}
        </article>

          </div>
        </section>

        <section className="diagnostics-group">
          <h2 className="diagnostics-group-title">
            {uiText("Gameplay readiness", language)}
          </h2>

          <div className="diagnostics-grid diagnostics-gameplay-grid">
        <article
          className={`status-card ${
            status.teleportsReady
              ? ""
              : "wide optional-card"
          }`}
        >
          <div className="status-heading">
            <StatusDot
              tone="info"
            />

            <span>
              {uiText("Teleport helper", language)}
            </span>
            <DiagnosticInfo language={language} text={"Optional teleport point preparation. SPLIT normally prepares these automatically when needed."} />
          </div>

          <strong>{uiText("Optional feature", language)}</strong>

          {!status.teleportsReady && (
            <div className="diagnostic-details">
              <p className="diagnostic-description">
                {runtimeUiText("SPLIT prepares teleport points automatically when you load a populated slot.", language)}
              </p>

              <p className="diagnostic-description teleport-reassurance">
                {runtimeUiText("You do not need to do anything here for Save / Load to work.", language)}
              </p>

              <div className="diagnostic-fix">
                <span>
                  {runtimeUiText("PREPARE MANUALLY", language)}
                </span>

                <p className="teleport-manual-intro">
                  {runtimeUiText("Use this only if you want the teleport points to be ready in advance.", language)}
                </p>

                <ol>
                  <li>
                    {uiText("Make sure Deadlock is running.", language)}
                  </li>

                  <li>
                    {uiText("Enter \"Explore NYC\".", language)}
                  </li>

                  <li>
                    {uiText("Click Prepare now.", language)}
                  </li>
                </ol>
              </div>

              <p className="teleport-context-note">
                {runtimeUiText("Teleport points may need to be prepared again after startup repair, saving a slot, switching presets, or changing the active slot bank.", language)}
              </p>

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
                    ? runtimeUiText("Preparing…", language)
                    : runtimeUiText("Prepare now", language)}
                </button>

                {!status.deadlockRunning && (
                  <span className="diagnostic-action-hint">
                    {uiText("Start Deadlock first.", language)}
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
              {uiText("Presentation mask", language)}
            </span>
            <DiagnosticInfo language={language} text={"Optional visual support during position loading."} />
          </div>

          <strong>
            {status.presentationMaskActive
              ? runtimeUiText("Active", language)
              : runtimeUiText("Normal", language)}
          </strong>

          {status.presentationMaskActive && (
            <div className="diagnostic-details">
              <p className="diagnostic-description">
                {runtimeUiText("SPLIT believes Deadlock presentation is still paused by r_force_no_present.", language)}
              </p>

              <p className="diagnostic-description">
                {runtimeUiText("Deadlock may appear frozen even though the game process is still running normally.", language)}
              </p>

              <div className="diagnostic-fix">
                <span>
                  {runtimeUiText("HOW TO FIX", language)}
                </span>

                <ol>
                  <li>
                    {uiText("Click Resume presentation below.", language)}
                  </li>

                  <li>
                    {runtimeUiText("If automatic recovery fails, bring Deadlock to the foreground.", language)}
                  </li>

                  <li>
                    {runtimeUiText("Press F10 manually. During an active presentation mask, F10 is SPLIT's emergency recovery key.", language)}
                  </li>

                  <li>
                    {runtimeUiText("If this happens repeatedly, copy the diagnostic report and report the issue.", language)}
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
                    ? runtimeUiText("Resuming…", language)
                    : runtimeUiText("Resume presentation", language)}
                </button>

                {!status.deadlockRunning && (
                  <span className="diagnostic-action-hint">
                    {uiText("Deadlock is not running.", language)}
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
                  ? "off"
                  : status.cameraRuntimeReady
                    ? "ok"
                    : "error"
              }
            />

            <span>
              {uiText("Camera runtime", language)}
            </span>
            <DiagnosticInfo language={language} text={"Checks camera-related runtime support used by position operations."} />
          </div>

          <strong>
            {!status.cameraRuntimeChecked
              ? runtimeUiText("Not tested", language)
              : status.cameraRuntimeReady
                ? runtimeUiText("Ready", language)
                : runtimeUiText("Unavailable", language)}
          </strong>

          {!status.cameraRuntimeReady && (
            <div className="diagnostic-details">
              {!status.cameraRuntimeChecked ? (
                <>
                  <p className="diagnostic-description">
                    {runtimeUiText("The camera runtime has not been checked during this Deadlock session yet.", language)}
                  </p>

                  <p className="diagnostic-description">
                    {runtimeUiText("Start Deadlock and enter Sandbox / Practice mode, then test the camera.", language)}
                  </p>
                </>
              ) : (
                <>
                  <div className="diagnostic-reason">
                    <span>
                      {runtimeUiText("REASON", language)}
                    </span>

                    <code>
                      {status.cameraRuntimeError ??
                        runtimeUiText("Unknown camera runtime error", language)}
                    </code>
                  </div>

                  <div className="diagnostic-fix">
                    <span>
                      {runtimeUiText("HOW TO FIX", language)}
                    </span>

                    <ol>
                      <li>
                        {uiText("Make sure Deadlock is running.", language)}
                      </li>

                      <li>
                        {runtimeUiText("Enter Sandbox or Practice mode so the in-game camera is active.", language)}
                      </li>

                      <li>
                        {uiText("Click Retry camera below.", language)}
                      </li>

                      <li>
                        {runtimeUiText("If the problem started after a Deadlock update, update SPLIT to the latest version.", language)}
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
                    ? runtimeUiText("Testing…", language)
                    : status.cameraRuntimeChecked
                      ? runtimeUiText("Retry camera", language)
                      : runtimeUiText("Test camera", language)}
                </button>

                {!status.deadlockRunning && (
                  <span className="diagnostic-action-hint">
                    {uiText("Start Deadlock first.", language)}
                  </span>
                )}
              </div>
            </div>
          )}
        </article>

          </div>
        </section>

        <section className="diagnostics-group">
          <h2 className="diagnostics-group-title">
            {uiText("Configuration", language)}
          </h2>

          <div className="diagnostics-grid diagnostics-configuration-grid">

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={Boolean(status.deadlockPath)}
            />

            <span>
              {uiText("Game folder", language)}
            </span>
            <DiagnosticInfo language={language} text={"The detected Deadlock installation path."} />
          </div>

          <code
            className="diagnostic-path"
            title={status.deadlockPath ?? runtimeUiText("Not configured", language)}
          >
            {status.deadlockPath ?? runtimeUiText("Not configured", language)}
          </code>
        </article>

        <article className="status-card">
          <div className="status-heading">
            <StatusDot
              ok={status.cfgDirExists}
            />

            <span>{runtimeUiText("CFG directory", language)}</span>
            <DiagnosticInfo language={language} text={"Folder containing Deadlock configuration scripts."} />
          </div>

          <strong>
            {status.cfgDirExists
              ? runtimeUiText("Ready", language)
              : runtimeUiText("Missing", language)}
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
            <DiagnosticInfo language={language} text={"Game script used for SPLIT's save/load integration."} />
          </div>

          <strong>
            {status.savestateCfgValid
              ? runtimeUiText("Valid", language)
              : status.savestateCfgExists
                ? runtimeUiText("Invalid", language)
                : runtimeUiText("Missing", language)}
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
            <DiagnosticInfo language={language} text={"Game script used to prepare position loading."} />
          </div>

          <strong>
            {status.prepareCfgValid
              ? runtimeUiText("Valid", language)
              : status.prepareCfgExists
                ? runtimeUiText("Invalid", language)
                : runtimeUiText("Missing", language)}
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
            <DiagnosticInfo language={language} text={"Deadlock startup configuration used to initialize the integration."} />
          </div>

          <strong>
            {status.autoexecValid
              ? runtimeUiText("Configured", language)
              : status.autoexecExists
                ? runtimeUiText("Missing SPLIT entry", language)
                : runtimeUiText("Missing", language)}
          </strong>
        </article>

          </div>
        </section>

        <section className="diagnostics-group">
          <h2 className="diagnostics-group-title">
            {uiText("Debug data", language)}
          </h2>

          <div className="diagnostics-grid diagnostics-debug-grid">
        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot
              ok={status.consoleLogExists}
            />

            <span>
              {uiText("Console log", language)}
            </span>
            <DiagnosticInfo language={language} text={"Game console output that SPLIT monitors for events and positions."} />
          </div>

          <code
            className="diagnostic-path"
            title={status.consoleLogPath ?? runtimeUiText("No Deadlock path available", language)}
          >
            {status.consoleLogPath ??
              runtimeUiText("No Deadlock path available", language)}
          </code>
        </article>

        <article className="status-card wide">
          <div className="status-heading">
            <StatusDot
              ok={Boolean(lastPosition)}
            />

            <span>
              {uiText("Last parsed position", language)}
            </span>
            <DiagnosticInfo language={language} text={"Most recent coordinates successfully read from Deadlock's console log."} />
          </div>

          {lastPosition ? (
            <code className="diagnostic-technical-value">
              XYZ {lastPosition.x}{" "}
              {lastPosition.y}{" "}
              {lastPosition.z} · ANG{" "}
              {lastPosition.pitch}{" "}
              {lastPosition.yaw}{" "}
              {lastPosition.roll}
            </code>
          ) : (
            <strong>
              {uiText("Waiting for a new getpos response…", language)}
            </strong>
          )}
        </article>
          </div>
        </section>
      </section>
    )}

    {activeSettingsSection === "about" && (
      <section className="about-settings-section" aria-labelledby="about-settings-title">
        <div className="about-settings-heading">
          <p className="label">{runtimeUiText("ABOUT", language)}</p>
          <h2 id="about-settings-title">SPLIT</h2>
        </div>
        <button type="button" className="preset-button about-replay-guide" onClick={() => setShowOnboarding(true)}>{t("replayGuide", language)}</button>
        <div className="about-settings-list">
          <div className="about-settings-row">
            <span>{runtimeUiText("Version", language)}</span>
            <strong>{appVersion ?? runtimeUiText("Loading…", language)}</strong>
          </div>
        </div>
      </section>
    )}
      </div>
      )}

      {showOnboarding && <Onboarding onClose={() => setShowOnboarding(false)} navigate={navigateOnboarding} />}
      {error && (
        <div className="error-box">
          {runtimeUiText("Backend error: ", language)}{error}
        </div>
      )}

      
      </main>
    </div>
  );
}

export default App;
