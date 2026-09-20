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

import "./quick-access.css";


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


type Hotkey = {
  key: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
};


type QuickHotkeys = {
  loadSlots: Array<Hotkey>;
  saveSlots: Array<Hotkey>;
  quickAccess: Hotkey;
};


function formatHotkey(
  hotkey: Hotkey | undefined,
): string {
  if (!hotkey) {
    return "";
  }

  return [
    hotkey.ctrl
      ? "Ctrl"
      : null,

    hotkey.alt
      ? "Alt"
      : null,

    hotkey.shift
      ? "Shift"
      : null,

    hotkey.key,
  ]
    .filter(Boolean)
    .join(" + ");
}


function matchesHotkey(
  event: KeyboardEvent,
  hotkey: Hotkey | null,
): boolean {
  if (!hotkey) {
    return false;
  }

  const key =
    event.key.length === 1
      ? event.key.toUpperCase()
      : event.key;

  return (
    key === hotkey.key &&
    event.ctrlKey === hotkey.ctrl &&
    event.altKey === hotkey.alt &&
    event.shiftKey === hotkey.shift
  );
}


export default function QuickAccess() {
  const [
    slots,
    setSlots,
  ] = useState<
    Array<PositionSnapshot | null>
  >(
    Array.from(
      { length: 8 },
      () => null,
    ),
  );

  const [
    metadata,
    setMetadata,
  ] = useState<Array<SlotMetadata>>([]);

  const [
    bankName,
    setBankName,
  ] = useState("Preset");

  const [
    hotkeys,
    setHotkeys,
  ] = useState<QuickHotkeys | null>(
    null,
  );

  const [
    workingSlot,
    setWorkingSlot,
  ] = useState<number | null>(
    null,
  );

  const [
    error,
    setError,
  ] = useState<string | null>(
    null,
  );


  const refresh =
    useCallback(async () => {
      try {
        const [
          nextSlots,
          nextMetadata,
          activePreset,
          presetNames,
          favoriteMode,
          nextHotkeys,
        ] =
          await Promise.all([
            invoke<
              Array<
                PositionSnapshot | null
              >
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

            invoke<boolean>(
              "get_favorite_mode",
            ),

            invoke<QuickHotkeys>(
              "get_hotkey_settings",
            ),
          ]);

        setSlots(
          nextSlots,
        );

        setMetadata(
          nextMetadata,
        );

        setHotkeys(
          nextHotkeys,
        );

        setBankName(
          favoriteMode
            ? "Favorites"
            : (
                presetNames[
                  activePreset - 1
                ] ??
                `Preset ${activePreset}`
              ),
        );

        setError(null);
      } catch (reason) {
        setError(
          String(reason),
        );
      }
    }, []);


  const close =
    useCallback(async () => {
      try {
        await invoke(
          "hide_quick_access",
        );
      } catch (reason) {
        console.error(
          reason,
        );
      }
    }, []);


  useEffect(() => {
    void refresh();

    let unlisten:
      | (() => void)
      | undefined;

    void listen(
      "quick-access-refresh",
      () => {
        void refresh();
      },
    ).then((cleanup) => {
      unlisten =
        cleanup;
    });

    return () => {
      unlisten?.();
    };
  }, [refresh]);


  useEffect(() => {
    const onKeyDown =
      (
        event: KeyboardEvent,
      ) => {
        if (
          event.key ===
            "Escape" ||
          matchesHotkey(
            event,
            hotkeys?.quickAccess ??
              null,
          )
        ) {
          event.preventDefault();

          void close();
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
  }, [
    close,
    hotkeys,
  ]);


  const activateSlot =
    useCallback(
      async (
        index: number,
      ) => {
        if (
          workingSlot !==
          null
        ) {
          return;
        }

        const slot =
          index + 1;

        const position =
          slots[index];

        setWorkingSlot(
          slot,
        );

        setError(null);

        try {
          if (position) {
            await invoke(
              "load_slot",
              { slot },
            );
          } else {
            await invoke(
              "capture_slot",
              { slot },
            );
          }

          await invoke(
            "hide_quick_access",
          );
        } catch (reason) {
          setError(
            String(reason),
          );
        } finally {
          setWorkingSlot(
            null,
          );
        }
      },
      [
        slots,
        workingSlot,
      ],
    );


  return (
    <main className="quick-access-shell">
      <header className="quick-access-header">
        <div>
          <span>
            SPLIT
          </span>

          <strong>
            {bankName}
          </strong>
        </div>

        <button
          type="button"
          onClick={() =>
            void close()
          }
          aria-label="Close Quick Access"
        >
          ×
        </button>
      </header>


      <div className="quick-access-list">
        {slots.map(
          (
            position,
            index,
          ) => {
            const slot =
              index + 1;

            const info =
              metadata[index];

            const name =
              info?.name?.trim() ||
              `Slot ${slot}`;

            const hotkey =
              position
                ? hotkeys
                    ?.loadSlots[
                      index
                    ]
                : hotkeys
                    ?.saveSlots[
                      index
                    ];

            return (
              <button
                key={slot}
                type="button"
                className={`quick-access-slot ${
                  position
                    ? "filled"
                    : "empty"
                }`}
                disabled={
                  workingSlot !==
                  null
                }
                style={
                  info?.color
                    ? {
                        borderLeftColor:
                          info.color,
                      }
                    : undefined
                }
                onClick={() =>
                  void activateSlot(
                    index,
                  )
                }
              >
                <span className="quick-access-slot-number">
                  {String(
                    slot,
                  ).padStart(
                    2,
                    "0",
                  )}
                </span>


                <div className="quick-access-preview">
                  {position &&
                  info?.screenshot ? (
                    <img
                      src={convertFileSrc(
                        info.screenshot,
                      )}
                      alt=""
                      draggable={
                        false
                      }
                    />
                  ) : position ? (
                    <span>
                      SAVED
                    </span>
                  ) : (
                    <span>
                      +
                    </span>
                  )}
                </div>


                <div className="quick-access-slot-copy">
                  <strong>
                    {name}
                  </strong>

                  <span>
                    {workingSlot ===
                    slot
                      ? position
                        ? "Loading…"
                        : "Saving…"
                      : position
                        ? "Load position"
                        : "Save current position"}
                  </span>
                </div>


                <kbd>
                  {formatHotkey(
                    hotkey,
                  )}
                </kbd>
              </button>
            );
          },
        )}
      </div>


      <footer className="quick-access-footer">
        {error ? (
          <span className="quick-access-error">
            {error}
          </span>
        ) : (
          <>
            <span>
                CapsLock
            </span>

            <span>
              or Esc to close
            </span>
          </>
        )}
      </footer>
    </main>
  );
}