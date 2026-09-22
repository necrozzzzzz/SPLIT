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
  listen,
} from "@tauri-apps/api/event";

import {
  getFullScreenshotPath,
} from "./screenshot";

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


type QuickAccessState = {
  visible: boolean;
  interactive: boolean;
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

  const [
    interactionMode,
    setInteractionMode,
  ] = useState(false);

  const [
    viewerSlot,
    setViewerSlot,
  ] = useState<number | null>(null);

  const [
    editingSlot,
    setEditingSlot,
  ] = useState<number | null>(null);

  const [
    editingName,
    setEditingName,
  ] = useState("");

  const [
    openMenuSlot,
    setOpenMenuSlot,
  ] = useState<number | null>(null);

  const [
    confirmClearSlot,
    setConfirmClearSlot,
  ] = useState<number | null>(null);

  const renameInputRef =
    useRef<HTMLInputElement | null>(null);

  const committingRenameRef =
    useRef<number | null>(null);

  const suppressCardClickRef =
    useRef(false);

  const textInputActiveRef =
    useRef(false);

  const viewerOpenRef =
    useRef(false);


  const closeSlotMenu =
    useCallback(() => {
      setOpenMenuSlot(null);
      setConfirmClearSlot(null);
    }, []);


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


  useEffect(() => {
    void refresh();

    let disposed = false;
    const cleanups: Array<() => void> = [];

    const refreshEvents = [
      "quick-access-refresh",
      "deadlock-slots",
      "deadlock-preset",
      "deadlock-favorite-mode",
    ];

    void (async () => {
      for (const event of refreshEvents) {
        try {
          const cleanup = await listen(
            event,
            () => {
              closeSlotMenu();
              void refresh();
            },
          );

          if (disposed) {
            cleanup();
          } else {
            cleanups.push(
              cleanup,
            );
          }
        } catch (reason) {
          console.error(
            `[SPLIT][QA] Could not initialize ${event} listener:`,
            reason,
          );
        }
      }
    })();

    return () => {
      disposed = true;
      cleanups.forEach(
        (cleanup) => cleanup(),
      );
    };
  }, [closeSlotMenu, refresh]);


  useEffect(() => {
    let disposed = false;
    let receivedLiveEvent = false;
    let unlisten:
      | (() => void)
      | undefined;

    void (async () => {
      try {
        const cleanup = await listen<boolean>(
          "quick-access-interaction",
          (event) => {
            receivedLiveEvent = true;
            setInteractionMode(
              event.payload,
            );
          },
        );

        if (disposed) {
          cleanup();
          return;
        }

        unlisten = cleanup;

        const state = await invoke<QuickAccessState>(
          "get_quick_access_state",
        );

        if (
          !disposed &&
          !receivedLiveEvent
        ) {
          setInteractionMode(
            state.interactive,
          );
        }
      } catch (reason) {
        console.error(
          "[SPLIT][QA] Could not initialize interaction listener:",
          reason,
        );
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);


  useEffect(() => {
    if (editingSlot === null) {
      return;
    }

    renameInputRef.current?.focus();
    renameInputRef.current?.select();
  }, [editingSlot]);


  const beginRename =
    useCallback(
      async (
        slot: number,
        name: string,
      ) => {
        console.log(
          "[SPLIT][QA][UI] rename begin",
        );
        setError(null);

        try {
          console.log(
            "[SPLIT][QA][UI] suspending native hotkeys",
          );
          await invoke(
            "set_quick_access_text_input_active",
            { active: true },
          );
          console.log(
            "[SPLIT][QA][UI] native hotkeys suspended",
          );
          textInputActiveRef.current = true;
          setEditingName(name);
          setEditingSlot(slot);
        } catch (reason) {
          setError(String(reason));
        }
      },
      [],
    );


  const finishRename =
    useCallback(
      async (
        slot: number,
        previousName: string,
        save: boolean,
      ) => {
        if (committingRenameRef.current !== null) {
          return;
        }

        const requestedName =
          editingName;

        committingRenameRef.current = slot;
        setEditingSlot(null);
        setError(null);
        let renamed = false;

        try {
          if (
            save &&
            requestedName.trim() !==
              previousName.trim()
          ) {
            await invoke(
              "rename_slot",
              {
                slot,
                name: requestedName,
              },
            );

            setMetadata(
              (current) =>
                current.map(
                  (item, index) =>
                    index === slot - 1
                      ? {
                          ...item,
                          name:
                            requestedName.trim(),
                        }
                      : item,
                ),
            );
            renamed = true;
          } else if (!save) {
            setEditingName(previousName);
          }
        } catch (reason) {
          setError(
            String(reason),
          );
        } finally {
          try {
            await invoke(
              "set_quick_access_text_input_active",
              { active: false },
            );
            textInputActiveRef.current = false;
          } catch (reason) {
            setError(String(reason));
          }

          committingRenameRef.current =
            null;
        }

        if (renamed) {
          await refresh();
        }
      },
      [
        editingName,
        refresh,
      ],
    );


  const openViewer =
    useCallback(
      async (slot: number) => {
        setError(null);

        try {
          await invoke(
            "set_quick_access_viewer_open",
            { open: true },
          );
          viewerOpenRef.current = true;
          setViewerSlot(slot);
        } catch (reason) {
          setError(String(reason));
        }
      },
      [],
    );


  const closeViewer =
    useCallback(async () => {
      setViewerSlot(null);

      try {
        await invoke(
          "set_quick_access_viewer_open",
          { open: false },
        );
        viewerOpenRef.current = false;
      } catch (reason) {
        setError(String(reason));
      }
    }, []);


  useEffect(() => {
    if (interactionMode) {
      return;
    }

    closeSlotMenu();

    if (
      editingSlot !== null &&
      committingRenameRef.current === null
    ) {
      const previousName =
        metadata[editingSlot - 1]?.name ??
        `Slot ${editingSlot}`;
      void finishRename(
        editingSlot,
        previousName,
        false,
      );
    }

    if (viewerOpenRef.current) {
      void closeViewer();
    }
  }, [
    closeViewer,
    closeSlotMenu,
    editingSlot,
    finishRename,
    interactionMode,
    metadata,
  ]);


  useEffect(() => () => {
    if (textInputActiveRef.current) {
      void invoke(
        "set_quick_access_text_input_active",
        { active: false },
      );
    }

    if (viewerOpenRef.current) {
      void invoke(
        "set_quick_access_viewer_open",
        { open: false },
      );
    }
  }, []);


  const activateSlot =
    useCallback(
      async (
        index: number,
        overwrite = false,
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
          if (position && !overwrite) {
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


  const updateSlotColor =
    useCallback(
      async (
        slot: number,
        color: string | null,
      ) => {
        if (workingSlot !== null) {
          return;
        }

        setWorkingSlot(slot);
        setError(null);

        try {
          await invoke(
            "set_slot_color",
            { slot, color },
          );

          setMetadata((current) =>
            current.map((item, index) =>
              index === slot - 1
                ? {
                    ...item,
                    color,
                  }
                : item,
            ),
          );
          closeSlotMenu();
          await refresh();
        } catch (reason) {
          setError(String(reason));
        } finally {
          setWorkingSlot(null);
        }
      },
      [
        closeSlotMenu,
        refresh,
        workingSlot,
      ],
    );


  const clearSlot =
    useCallback(
      async (slot: number) => {
        if (workingSlot !== null) {
          return;
        }

        setWorkingSlot(slot);
        setError(null);

        try {
          await invoke(
            "clear_slot",
            { slot },
          );
          closeSlotMenu();
          await refresh();
        } catch (reason) {
          setError(String(reason));
        } finally {
          setWorkingSlot(null);
        }
      },
      [
        closeSlotMenu,
        refresh,
        workingSlot,
      ],
    );

  const viewerMetadata =
    viewerSlot === null
      ? null
      : metadata[viewerSlot - 1];

  const viewerThumbnailPath =
    viewerMetadata?.screenshot ?? null;

  const viewerFullPath =
    viewerThumbnailPath
      ? getFullScreenshotPath(
          viewerThumbnailPath,
        )
      : null;

  const viewerName =
    viewerSlot === null
      ? ""
      : (
          viewerMetadata?.name?.trim() ||
          `Slot ${viewerSlot}`
        );


  return (
    <main
      className={`quick-access-shell${
        interactionMode
          ? " interactive"
          : ""
      }`}
      onContextMenu={(event) => {
        event.preventDefault();
      }}
      onClick={() => {
        closeSlotMenu();
      }}
    >
      <header className="quick-access-header">
        <span className="quick-access-brand">
            SPLIT
        </span>

        <strong>
            {bankName}
        </strong>
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
              <div
                key={slot}
                role="button"
                tabIndex={0}
                className={`quick-access-slot ${
                  position
                    ? "filled"
                    : "empty"
                }${
                  openMenuSlot === slot
                    ? " menu-open"
                    : ""
                }`}
                aria-disabled={
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
                onClick={() => {
                  if (
                    suppressCardClickRef.current ||
                    workingSlot !== null
                  ) {
                    suppressCardClickRef.current =
                      false;
                    return;
                  }

                  if (openMenuSlot !== null) {
                    closeSlotMenu();
                    return;
                  }

                  void activateSlot(
                    index,
                  );
                }}
                onKeyDown={(event) => {
                  if (
                    event.target !==
                      event.currentTarget ||
                    workingSlot !== null ||
                    (
                      event.key !== "Enter" &&
                      event.key !== " "
                    )
                  ) {
                    return;
                  }

                  event.preventDefault();
                  void activateSlot(
                    index,
                  );
                }}
              >
                <span className="quick-access-slot-number">
                    {String(
                        slot,
                    ).padStart(
                        2,
                        "0",
                    )}
                    </span>


                    {position ? (
                    <div className="quick-access-preview">
                        {info?.screenshot ? (
                        <img
                            src={convertFileSrc(
                            info.screenshot,
                            )}
                            alt=""
                            draggable={false}
                            title="Right click to view screenshot"
                            onContextMenu={(event) => {
                              event.preventDefault();
                              event.stopPropagation();
                              void openViewer(slot);
                            }}
                        />
                        ) : (
                        <span>
                            SAVED
                        </span>
                        )}
                    </div>
                    ) : (
                    <div className="quick-access-empty-mark">
                        +
                    </div>
                    )}


                    <div className="quick-access-slot-copy">
                    <div className="quick-access-slot-title-row">
                    {editingSlot === slot ? (
                      <input
                        ref={renameInputRef}
                        className="quick-access-slot-name-input"
                        value={editingName}
                        aria-label={`Rename slot ${slot}`}
                        onChange={(event) =>
                          setEditingName(
                            event.target.value,
                          )
                        }
                        onClick={(event) =>
                          event.stopPropagation()
                        }
                        onPointerDown={(event) =>
                          event.stopPropagation()
                        }
                        onKeyDown={(event) => {
                          event.stopPropagation();

                          if (event.key === "Enter") {
                            event.preventDefault();
                            void finishRename(
                              slot,
                              name,
                              true,
                            );
                          } else if (
                            event.key === "Escape"
                          ) {
                            event.preventDefault();
                            void finishRename(
                              slot,
                              name,
                              false,
                            );
                          }
                        }}
                        onBlur={() => {
                          suppressCardClickRef.current =
                            true;
                          window.setTimeout(() => {
                            suppressCardClickRef.current =
                              false;
                          }, 0);

                          void finishRename(
                            slot,
                            name,
                            true,
                          );
                        }}
                      />
                    ) : (
                      <button
                        type="button"
                        className="quick-access-slot-name"
                        onClick={(event) => {
                          event.stopPropagation();
                          void beginRename(
                            slot,
                            name,
                          );
                        }}
                      >
                        {name}
                      </button>
                    )}

                    <div className="quick-access-slot-controls">
                    {interactionMode && position && (
                      <div
                        className="quick-access-slot-actions"
                        onClick={(event) => {
                          event.stopPropagation();
                        }}
                        onPointerDown={(event) => {
                          event.stopPropagation();
                        }}
                      >
                        <button
                          type="button"
                          className="quick-access-slot-actions-button"
                          aria-label={`Slot ${slot} actions`}
                          aria-haspopup="menu"
                          aria-expanded={
                            openMenuSlot === slot
                          }
                          disabled={workingSlot !== null}
                          onClick={(event) => {
                            event.stopPropagation();
                            setConfirmClearSlot(null);
                            setOpenMenuSlot(
                              (current) =>
                                current === slot
                                  ? null
                                  : slot,
                            );
                          }}
                        >
                          ...
                        </button>

                        {openMenuSlot === slot && (
                          <div
                            className={`quick-access-slot-menu${
                              slot > 5
                                ? " opens-up"
                                : ""
                            }`}
                            role="menu"
                            aria-label={`Slot ${slot} actions`}
                            onClick={(event) => {
                              event.stopPropagation();
                            }}
                          >
                            <button
                              type="button"
                              role="menuitem"
                              className="quick-access-slot-menu-item"
                              disabled={workingSlot !== null}
                              onClick={(event) => {
                                event.stopPropagation();
                                closeSlotMenu();
                                void activateSlot(
                                  index,
                                  true,
                                );
                              }}
                            >
                              Overwrite
                            </button>

                            <div className="quick-access-slot-menu-divider" />

                            <span className="quick-access-slot-menu-label">
                              Color
                            </span>

                            <div className="quick-access-slot-menu-colors">
                              {SLOT_COLORS.map(
                                ({ label, value }) => {
                                  const active =
                                    (info?.color ?? null) ===
                                    value;

                                  return (
                                    <button
                                      key={label}
                                      type="button"
                                      role="menuitemradio"
                                      aria-checked={active}
                                      aria-label={`${label} color for slot ${slot}`}
                                      title={label}
                                      className={`quick-access-slot-menu-color${
                                        active
                                          ? " active"
                                          : ""
                                      }`}
                                      disabled={workingSlot !== null}
                                      onClick={(event) => {
                                        event.stopPropagation();
                                        void updateSlotColor(
                                          slot,
                                          value,
                                        );
                                      }}
                                    >
                                      {value ? (
                                        <span
                                          className="quick-access-slot-menu-swatch"
                                          style={{
                                            backgroundColor:
                                              value,
                                          }}
                                        />
                                      ) : (
                                        <span className="quick-access-slot-menu-none">
                                          /
                                        </span>
                                      )}
                                    </button>
                                  );
                                },
                              )}
                            </div>

                            <div className="quick-access-slot-menu-divider" />

                            <button
                              type="button"
                              role="menuitem"
                              className={`quick-access-slot-menu-item danger${
                                confirmClearSlot === slot
                                  ? " confirming"
                                  : ""
                              }`}
                              disabled={workingSlot !== null}
                              onClick={(event) => {
                                event.stopPropagation();

                                if (
                                  confirmClearSlot !== slot
                                ) {
                                  setConfirmClearSlot(slot);
                                  return;
                                }

                                void clearSlot(slot);
                              }}
                            >
                              {confirmClearSlot === slot
                                ? "Confirm clear"
                                : "Clear"}
                            </button>
                          </div>
                        )}
                      </div>
                    )}

                    <kbd>
                      {formatHotkey(
                        hotkey,
                      )}
                    </kbd>
                    </div>
                    </div>

                    <span>
                        {workingSlot === slot
                        ? position
                            ? "Loading…"
                            : "Saving…"
                        : position
                            ? "Load position"
                            : "Save current position"}
                    </span>
                    </div>


              </div>
            );
          },
        )}
      </div>


      <footer className="quick-access-footer">
        {error ? (
          <span className="quick-access-error">
            {error}
          </span>
        ) : interactionMode ? (
          <>
            <span className="quick-access-interaction-label">
              INTERACTION MODE
            </span>

            <span className="quick-access-footer-key">
              CapsLock / Esc
            </span>

            <span>
              Back to game
            </span>
          </>
        ) : (
          <>
            <span className="quick-access-footer-key">
                {formatHotkey(
                hotkeys?.quickAccess,
                )}
            </span>

            <span>
                Close
            </span>

            <span className="quick-access-footer-hint">
                Hold CapsLock to interact
            </span>

            <span className="quick-access-footer-key">
                Esc
            </span>

            <span>
                Close
            </span>
          </>
        )}
      </footer>


      {viewerSlot !== null &&
        viewerThumbnailPath && (
          <div
            className="quick-access-viewer-backdrop"
            onClick={() => {
              void closeViewer();
            }}
          >
            <section
              className="quick-access-viewer"
              role="dialog"
              aria-modal="true"
              aria-label={`Screenshot for ${viewerName}`}
            >
              <header
                className="quick-access-viewer-header"
                onClick={(event) =>
                  event.stopPropagation()
                }
              >
                <strong>
                  SAVE {viewerSlot} / {viewerName}
                </strong>

                <button
                  type="button"
                  aria-label="Close screenshot"
                  title="Close"
                  onClick={() => {
                    void closeViewer();
                  }}
                >
                  ×
                </button>
              </header>

              <div className="quick-access-viewer-image">
                <img
                  key={viewerThumbnailPath}
                  src={convertFileSrc(
                    viewerFullPath ??
                      viewerThumbnailPath,
                  )}
                  alt={`Screenshot for ${viewerName}`}
                  draggable={false}
                  onClick={(event) =>
                    event.stopPropagation()
                  }
                  onError={(event) => {
                    if (
                      viewerFullPath === null ||
                      event.currentTarget.dataset
                        .fallback === "true"
                    ) {
                      return;
                    }

                    event.currentTarget.dataset.fallback =
                      "true";
                    event.currentTarget.src =
                      convertFileSrc(
                        viewerThumbnailPath,
                      );
                  }}
                />
              </div>

              <footer
                className="quick-access-viewer-footer"
                onClick={(event) =>
                  event.stopPropagation()
                }
              >
                Click outside the image to close
              </footer>
            </section>
          </div>
        )}
    </main>
  );
}
