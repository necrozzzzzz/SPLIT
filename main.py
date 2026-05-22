import os
import sys
import re
import time
import threading
import shutil
import tkinter as tk
import psutil
import winsound
from tkinter import messagebox, filedialog, simpledialog
import keyboard
import json
import ctypes
import random
import ctypes
from PIL import Image, ImageTk, ImageFilter, ImageDraw, ImageFont

# Config

NUM_SLOTS = 8 
APP_VERSION = "1.0.0"
APP_NAME = "SPLIT"
SINGLE_INSTANCE_MUTEX_NAME = "SPLIT_DEADLOCK_SAVESTATE_MUTEX"
DEFAULT_STATUS = "Waiting for Deadlock..."
GAME_RUNNING_STATUS = "Game Running — Press F1–F8 to save..."
STATUS_MESSAGES = [
    "Game Running — Press F1–F8 to save.......................................",
    "........................................................The Lash approves........................................................",
    "........................................................Go play Makrill blyat....................................................",
    "........................................................Expelling goo............................................................",
    "........................................................Haze is missing..........................................................",
    "........................................................Arigato, Arighetto, uh-REG-eh-doe........................................",
    "........................................................Help me deliver this thing...............................................",
    "........................................................They're in mid...........................................................",
    "........................................................Let's go mid.............................................................",
    "........................................................Yellow needs help........................................................",
    "........................................................Broadway needs help......................................................",
    "........................................................Nebraska is on its way...................................................",
    "........................................................I like my pillow.........................................................",
    "........................................................Turn into frog...........................................................",
    "........................................................Why does bread taste good................................................"
]                      
APP_DATA_DIR = os.path.join(
    os.getenv("APPDATA"),
    "SPLIT"
)

os.makedirs(
    APP_DATA_DIR,
    exist_ok=True
)

SAVE_DATA_FILE = os.path.join(
    APP_DATA_DIR,
    "deadlock_savestate_slots.json"
)

APP_CONFIG_FILE = os.path.join(
    APP_DATA_DIR,
    "deadlock_savestate_config.json"
)
SAVE_KEY = "h"
SAVE_KEY_CFG = "h"
LOAD_KEYS = ["u", "i", "o", "j", "k", "l", "n", "m"]
DEFAULT_PRESET_CYCLE_KEY = "v"
DEFAULT_FAVORITE_MODE_HOTKEY = "f11"

DEFAULT_PRESET_HOTKEYS = [
    "ctrl+1",
    "ctrl+2",
    "ctrl+3",
    "ctrl+4"
]

RESERVED_HOTKEYS = [
    SAVE_KEY,
    *LOAD_KEYS,
    "f1", "f2", "f3", "f4",
    "f5", "f6", "f7", "f8",
    "alt+f1", "alt+f2", "alt+f3", "alt+f4",
    "alt+f5", "alt+f6", "alt+f7", "alt+f8",
]

# Deadlock paths (Steam default)
STEAM_DEFAULT = os.path.expandvars(
    r"%ProgramFiles(x86)%\Steam\steamapps\common\Deadlock"
)
CONSOLE_LOG = os.path.join(STEAM_DEFAULT, "game", "citadel", "console.log")
CFG_DIR     = os.path.join(STEAM_DEFAULT, "game", "citadel", "cfg")
CFG_FILE    = os.path.join(CFG_DIR, "savestate.cfg")
AUTOEXEC    = os.path.join(CFG_DIR, "autoexec.cfg")

def resource_path(relative_path):

    try:
        base_path = sys._MEIPASS

    except Exception:
        base_path = os.path.abspath(".")

    return os.path.join(base_path, relative_path)


def apply_window_icon(window):

    try:
        window.iconbitmap(
            resource_path("split.ico")
        )

    except Exception:
        pass

SETPOS_RE = re.compile(
    r"setpos(?:_exact)?\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)"
    r"(?:;setang(?:_exact)?\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+))?",
    re.IGNORECASE,
)


# State
class AppState:
    def __init__(self):
        self.preset_hotkeys = DEFAULT_PRESET_HOTKEYS.copy()
        self.undo_hotkey = "f9"
        self.redo_hotkey = "f10"
        self.last_action = None
        self.redo_action = None
        self.current_preset = 0
        self.preset_cycle_hotkey = DEFAULT_PRESET_CYCLE_KEY
        self.favorite_mode_hotkey = DEFAULT_FAVORITE_MODE_HOTKEY
        self.show_favorite_warning = True
        self.boot_sound_enabled = True

        self.favorite_mode = False
        self.favorite_slots = [None] * NUM_SLOTS
        self.favorite_names = [f"Favorite {i+1}" for i in range(NUM_SLOTS)]

        self.presets = [
            {
                "name": f"Preset {i+1}",
                "slots": [None] * NUM_SLOTS,
                "slot_names": [f"Slot {j+1}" for j in range(NUM_SLOTS)],
                "slot_times": [None] * NUM_SLOTS,
                "slot_colors": [None] * NUM_SLOTS
            }
            for i in range(4)
        ]

        self.slots = self.presets[self.current_preset]["slots"]
        self.slot_names = self.presets[self.current_preset]["slot_names"]
        self.slot_times = self.presets[self.current_preset]["slot_times"]
        self.slot_colors = self.presets[self.current_preset]["slot_colors"]
        self.pending_save_slot: int | None = None   # waiting for next getpos line
        self.log_thread: threading.Thread | None = None
        self.running = True
        self.deadlock_path = self.find_deadlock()
        self.status_msg = tk.StringVar(value=DEFAULT_STATUS)
        self.slot_vars: list[tk.StringVar] = [
            tk.StringVar(value="— empty —") for _ in range(NUM_SLOTS)
        ]
    
    def find_deadlock(self):

        def is_valid_deadlock_path(path):

            cfg_path = os.path.join(
                path,
                "game",
                "citadel",
                "cfg"
            )

            return os.path.exists(cfg_path)

        if os.path.exists(APP_CONFIG_FILE):

            with open(APP_CONFIG_FILE, "r", encoding="utf-8") as f:
                config = json.load(f)

            saved_path = config.get("deadlock_path")

            if saved_path and is_valid_deadlock_path(saved_path):
                return saved_path

        if is_valid_deadlock_path(STEAM_DEFAULT):

            use_default = ask_confirm_window(
                "Deadlock Folder Found",
                f"SPLIT found Deadlock here:\n\n{STEAM_DEFAULT}\n\nUse this folder?",
                confirm_text="Use Folder",
                danger=False,
                cancel_text="Browse",
                center_on_screen=True
            )

            if use_default:
                return STEAM_DEFAULT

        ask_confirm_window(
            "Game Not Found",
            "You MUST select the folder called \"Deadlock\"\n\n"

            "NOT:\n\n"

            "- Deadlock\\game\n"
            "- Deadlock\\game\\citadel\n\n"
            "Example:\n"
            "Steam\\steamapps\\common\\Deadlock",
            confirm_text="OK",
            danger=False,
            cancel_text="Browse",
            center_on_screen=True
        )

        while True:

            path = filedialog.askdirectory()

            if not path:
                sys.exit()

            if is_valid_deadlock_path(path):
                break

            retry = ask_confirm_window(
                "Invalid Folder",
                "You MUST select the folder called \"Deadlock\"\n\n"

                "NOT:\n\n"

                "- Deadlock\\game\n"
                "- Deadlock\\game\\citadel\n\n"

                "Example:\n\n"

                "Steam\\steamapps\\common\\Deadlock",
                confirm_text="Browse",
                danger=True,
                cancel_text="Cancel",
                center_on_screen=True
            )

            if not retry:
                sys.exit()

        with open(APP_CONFIG_FILE, "w", encoding="utf-8") as f:
            json.dump(
                {
                    "deadlock_path": path
                },
                f,
                indent=4
            )

        return path
    
    @property
    def console_log(self):
        return os.path.join(self.deadlock_path, "game", "citadel", "console.log")

    @property
    def cfg_dir(self):
        return os.path.join(self.deadlock_path, "game", "citadel", "cfg")

    @property
    def cfg_file(self):
        return os.path.join(self.cfg_dir, "savestate.cfg")

    @property
    def autoexec(self):
        return os.path.join(self.cfg_dir, "autoexec.cfg")




# CFG helpers
def write_cfg():

    """Write savestate.cfg with current active slots."""

    os.makedirs(state.cfg_dir, exist_ok=True)

    active_slots = state.favorite_slots if state.favorite_mode else state.slots

    lines = [
        f"// {APP_NAME} – auto-generated, do not edit manually\n",
        f"// {NUM_SLOTS} slots: F1-F{NUM_SLOTS} = save trigger | Alt+F1-Alt+F{NUM_SLOTS} = load\n\n",
    ]

    lines.append(
        'alias "savestate_getpos" "exec savestate; getpos_exact"\n'
    )

    lines.append(
        f'bind "{SAVE_KEY_CFG}" "savestate_getpos"\n\n'
    )

    for i, pos in enumerate(active_slots):

        if pos:
            lines.append(f'// Slot {i+1}: {pos}\n')
            lines.append(f'alias "load_slot_{i+1}" "{pos};noclip"\n')
            lines.append(f'bind "{LOAD_KEYS[i]}" "exec savestate; load_slot_{i+1}"\n')

        else:
            lines.append(f'// Slot {i+1}: empty\n')
            lines.append(f'alias "load_slot_{i+1}" "echo Slot {i+1} empty"\n')
            lines.append(f'bind "{LOAD_KEYS[i]}" "exec savestate; load_slot_{i+1}"\n')

    with open(state.cfg_file, "w", encoding="utf-8") as f:
        f.writelines(lines)


def backup_autoexec():

    if not os.path.exists(state.autoexec):
        return

    backup_path = state.autoexec + ".backup_deadlock_savestate"

    if os.path.exists(backup_path):
        return

    shutil.copy2(
        state.autoexec,
        backup_path
    )


def ensure_autoexec():

    """Make sure autoexec.cfg execs our savestate.cfg."""

    os.makedirs(state.cfg_dir, exist_ok=True)

    marker = "exec savestate"

    if os.path.exists(state.autoexec):

        content = open(
            state.autoexec,
            encoding="utf-8",
            errors="ignore"
        ).read()

        if marker in content:
            return

        backup_autoexec()

        with open(state.autoexec, "a", encoding="utf-8") as f:
            f.write(f"\n{marker}  // Added by {APP_NAME}\n")

    else:

        with open(state.autoexec, "w", encoding="utf-8") as f:
            f.write(f"{marker}  // Added by {APP_NAME}\n")

def backup_slots_json():

    if not os.path.exists(SAVE_DATA_FILE):
        return

    backup_file = os.path.join(
        APP_DATA_DIR,
        "deadlock_savestate_slots.backup.json"
    )

    shutil.copy2(
        SAVE_DATA_FILE,
        backup_file
    )

def save_slots_to_json():
    state.presets[state.current_preset]["slots"] = state.slots
    state.presets[state.current_preset]["slot_names"] = state.slot_names
    state.presets[state.current_preset]["slot_times"] = state.slot_times
    state.presets[state.current_preset]["slot_colors"] = state.slot_colors

    data = {
        "current_preset": state.current_preset,
        "presets": state.presets,
        "favorite_slots": list(state.favorite_slots),
        "favorite_names": state.favorite_names,
    }

    with open(SAVE_DATA_FILE, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=4)


def load_slots_from_json():
    if not os.path.exists(SAVE_DATA_FILE):
        return

    with open(SAVE_DATA_FILE, "r", encoding="utf-8") as f:
        data = json.load(f)

    if isinstance(data, list):
        state.presets[0]["slots"] = data
        state.presets[0]["slot_names"] = [
            f"Slot {i+1}" for i in range(NUM_SLOTS)
        ]
        state.current_preset = 0

    elif "presets" in data:
        state.current_preset = data.get("current_preset", 0)
        loaded_presets = data.get("presets", [])

        for i in range(4):
            if i < len(loaded_presets):
                state.presets[i]["name"] = loaded_presets[i].get(
                    "name",
                    f"Preset {i+1}"
                )
                state.presets[i]["slots"] = loaded_presets[i].get(
                    "slots",
                    [None] * NUM_SLOTS
                )
                state.presets[i]["slot_names"] = loaded_presets[i].get(
                    "slot_names",
                    [f"Slot {j+1}" for j in range(NUM_SLOTS)]
                )

                state.presets[i]["slot_times"] = loaded_presets[i].get(
                    "slot_times",
                    [None] * NUM_SLOTS
                )
                state.presets[i]["slot_colors"] = loaded_presets[i].get(
                    "slot_colors",
                    [None] * NUM_SLOTS
                )

    else:
        state.presets[0]["slots"] = data.get("slots", [])
        state.presets[0]["slot_names"] = data.get("slot_names", [])
        state.current_preset = 0

    state.current_preset = max(
        0,
        min(state.current_preset, 3)
    )

    state.slots = state.presets[state.current_preset]["slots"]
    state.slot_names = state.presets[state.current_preset]["slot_names"]
    state.slot_times = state.presets[state.current_preset]["slot_times"]
    state.slot_colors = state.presets[state.current_preset]["slot_colors"]

    favorites = data.get("favorite_slots", [None] * NUM_SLOTS)
    favorite_names = data.get(
        "favorite_names",
        [f"Favorite {i+1}" for i in range(NUM_SLOTS)]
    )

    state.favorite_slots = [
        favorites[i] if i < len(favorites) else None
        for i in range(NUM_SLOTS)
    ]

    state.favorite_names = [
        favorite_names[i] if i < len(favorite_names) else f"Favorite {i+1}"
        for i in range(NUM_SLOTS)
    ]

    refresh_slot_display()
    
def create_undo_snapshot():
    state.redo_action = None

    state.last_action = {
        "current_preset": state.current_preset,
        "slots": state.slots.copy(),
        "slot_names": state.slot_names.copy(),
        "slot_times": state.slot_times.copy(),
        "slot_colors": state.slot_colors.copy(),
        "preset_name": state.presets[
            state.current_preset
        ]["name"]
    }    
    
def format_saved_time(timestamp):

    if not timestamp:
        return ""

    elapsed = int(time.time() - timestamp)

    if elapsed < 60:
        return "Saved just now"

    minutes = elapsed // 60

    if minutes == 1:
        return "Saved 1 min ago"

    return f"Saved {minutes} min ago"    
    
def refresh_slot_display():

    active_slots = state.favorite_slots if state.favorite_mode else state.slots
    active_names = state.favorite_names if state.favorite_mode else state.slot_names

    for i in range(NUM_SLOTS):

        slot_bg = SLOT_FILLED if active_slots[i] else SLOT_EMPTY
    
        if active_slots[i]:

            label = active_names[i]

            saved_time = format_saved_time(state.slot_times[i])

            max_label_length = 22

            if saved_time:
                label = label[:max_label_length] + "…" if len(label) > max_label_length else label

            state.slot_vars[i].set(
                f"{label}  ·  {saved_time}"
            )

        else:
            state.slot_vars[i].set("— empty —")

        if i < len(slot_rows):

            row = slot_rows[i]

            row.configure(
                bg=slot_bg
            )

            if hasattr(row, "right_actions"):

                row.right_actions.configure(
                    bg=slot_bg
                )

            if hasattr(row, "favorite_btn"):

                row.favorite_btn.configure(
                    bg=slot_bg,
                    activebackground=slot_bg
                )

            if hasattr(row, "rename_btn"):

                row.rename_btn.configure(
                    bg=slot_bg,
                    activebackground=slot_bg
                )

            if hasattr(row, "delete_btn"):

                row.delete_btn.configure(
                    bg=slot_bg,
                    activebackground=slot_bg
                )

            if hasattr(row, "color_btn"):

                tag_color = state.slot_colors[i]

                row.color_btn.configure(
                    bg=tag_color if active_slots[i] and tag_color else slot_bg,
                    fg="white" if active_slots[i] and tag_color else MUTED,
                    activebackground=tag_color if active_slots[i] and tag_color else slot_bg,
                    activeforeground="white"
                )
            
def undo_last_action():
    if not state.last_action:
        show_notification(
            "⚠️ Nothing to undo",
            "warning"
        )

        reset_status_after_delay()
        return

    state.redo_action = {
        "current_preset": state.current_preset,
        "slots": state.slots.copy(),
        "slot_names": state.slot_names.copy(),
        "slot_times": state.slot_times.copy(),
        "slot_colors": state.slot_colors.copy(),
        "preset_name": state.presets[
            state.current_preset
        ]["name"]
    }

    snapshot = state.last_action
    preset_idx = snapshot["current_preset"]

    state.presets[preset_idx]["slots"] = snapshot["slots"].copy()
    state.presets[preset_idx]["slot_names"] = snapshot["slot_names"].copy()
    state.presets[preset_idx]["slot_times"] = snapshot["slot_times"].copy()
    state.presets[preset_idx]["slot_colors"] = snapshot["slot_colors"].copy()
    state.presets[preset_idx]["name"] = snapshot["preset_name"]

    state.current_preset = preset_idx
    state.slots = state.presets[preset_idx]["slots"]
    state.slot_names = state.presets[preset_idx]["slot_names"]
    state.slot_times = state.presets[preset_idx]["slot_times"]
    state.slot_colors = state.presets[preset_idx]["slot_colors"]

    refresh_slot_display()
    write_cfg()
    save_slots_to_json()

    try:
        update_preset_buttons()
    except Exception:
        pass

    show_notification(
        "↩️ Undo complete",
        "success"
    )

    reset_status_after_delay()

    state.last_action = None   

def redo_last_action():
    if not state.redo_action:
        show_notification(
            "⚠️ Nothing to redo",
            "warning"
        )

        reset_status_after_delay()      
        return

    state.last_action = {
        "current_preset": state.current_preset,
        "slots": state.slots.copy(),
        "slot_names": state.slot_names.copy(),
        "slot_times": state.slot_times.copy(),
        "slot_colors": state.slot_colors.copy(),
        "preset_name": state.presets[
            state.current_preset
        ]["name"]
    }

    snapshot = state.redo_action
    preset_idx = snapshot["current_preset"]

    state.presets[preset_idx]["slots"] = snapshot["slots"].copy()
    state.presets[preset_idx]["slot_names"] = snapshot["slot_names"].copy()
    state.presets[preset_idx]["slot_times"] = snapshot["slot_times"].copy()
    state.presets[preset_idx]["slot_colors"] = snapshot["slot_colors"].copy()
    state.presets[preset_idx]["name"] = snapshot["preset_name"]

    state.current_preset = preset_idx
    state.slots = state.presets[preset_idx]["slots"]
    state.slot_names = state.presets[preset_idx]["slot_names"]
    state.slot_times = state.presets[preset_idx]["slot_times"]
    state.slot_colors = state.presets[preset_idx]["slot_colors"]

    refresh_slot_display()
    write_cfg()
    save_slots_to_json()

    try:
        update_preset_buttons()
    except Exception:
        pass

    show_notification(
        "↪️ Redo complete",
        "success"
    )

    reset_status_after_delay()

    state.redo_action = None    
            
def switch_preset(preset_idx: int):
    state.presets[state.current_preset]["slots"] = state.slots
    state.presets[state.current_preset]["slot_names"] = state.slot_names
    state.presets[state.current_preset]["slot_times"] = state.slot_times
    state.presets[state.current_preset]["slot_colors"] = state.slot_colors

    state.current_preset = preset_idx

    state.slots = state.presets[preset_idx]["slots"]
    state.slot_names = state.presets[preset_idx]["slot_names"]
    state.slot_times = state.presets[preset_idx]["slot_times"]
    state.slot_colors = state.presets[preset_idx]["slot_colors"]

    refresh_slot_display()
    write_cfg()
    save_slots_to_json()

    show_notification(
        f"🔁 Switched to {state.presets[preset_idx]['name']}",
        "load"
    )

    reset_status_after_delay()

    try:
        update_preset_buttons()
    except Exception:
        pass            
            
def clear_current_preset_slots():
    confirm = ask_confirm_window(
        "Clear Slots",
        "Clear all slots in the current preset?",
        confirm_text="Clear Slots",
        danger=False
    )

    if not confirm:
        return

    backup_slots_json()

    create_undo_snapshot()

    for i in range(NUM_SLOTS):
        state.slots[i] = None
        state.slot_names[i] = f"Slot {i+1}"
        state.slot_times[i] = None
        state.slot_colors[i] = None        

    refresh_slot_display()
    
    write_cfg()
    save_slots_to_json()

    show_notification(
        "🗑️ All slots cleared",
        "error"
    )

    reset_status_after_delay()            
    
def reset_all_data():

    confirm = ask_confirm_window(
        "Reset All",
        "This will reset:\n\n"
        "• All presets\n"
        "• All slots\n"
        "• All favorites\n"
        "• All custom shortcuts\n"
        "• Startup sound and particles settings\n\n"
        "Your Deadlock folder path will be kept.\n\n"
        "Continue?",
        confirm_text="Reset Everything",
        danger=True
    )

    if not confirm:
        return

    backup_slots_json()

    # Reset presets
    state.presets = [
        {
            "name": f"Preset {i+1}",
            "slots": [None] * NUM_SLOTS,
            "slot_names": [f"Slot {j+1}" for j in range(NUM_SLOTS)],
            "slot_times": [None] * NUM_SLOTS,
            "slot_colors": [None] * NUM_SLOTS
        }
        for i in range(4)
    ]

    # Reset active preset
    state.current_preset = 0

    state.slots = state.presets[0]["slots"]

    state.slot_names = state.presets[0]["slot_names"]

    state.slot_times = state.presets[0]["slot_times"]

    state.slot_colors = state.presets[0]["slot_colors"]

    # Reset favorites
    state.favorite_slots = [None] * NUM_SLOTS
    

    state.favorite_names = [
        f"Favorite {i+1}"
        for i in range(NUM_SLOTS)
    ]

    # Reset hotkeys
    state.preset_cycle_hotkey = "v"
    state.undo_hotkey = "f9"
    state.redo_hotkey = "f10"
    state.favorite_mode_hotkey = "f11"

    # Reset modes
    state.favorite_mode = False
    
    # Reset options
    state.boot_sound_enabled = True

    global title_particles_enabled

    title_particles_enabled = True    

    # Refresh UI
    refresh_slot_display()

    write_cfg()

    save_slots_to_json()

    save_app_config()

    setup_hotkeys()

    try:
        update_preset_buttons()
    except Exception:
        pass

    show_notification(
        "🧹 Reset complete",
        "success"
    )

    reset_status_after_delay()    
            
def rename_current_preset():
    current_name = state.presets[state.current_preset]["name"]

    new_name = ask_text_window(
        "Rename Preset",
        "New preset name:",
        current_name
    )

    if not new_name:
        return

    state.presets[state.current_preset]["name"] = (
        new_name.strip()
    )

    save_slots_to_json()

    try:
        update_preset_buttons()
    except Exception:
        pass

    show_notification(
        "✏️ Preset renamed",
        "success"
    )

    reset_status_after_delay()            
    

OPTIONS_PANEL_WIDTH = 320
OPTIONS_PANEL_HEIGHT = 455

OPTIONS_OVERLAY_PADDING_X = 8
OPTIONS_OVERLAY_PADDING_Y = 12


def animate_open_options_panel(height=1):

    if not options_panel:
        return

    remaining = OPTIONS_PANEL_HEIGHT - height
    step = max(
        6,
        int(remaining * 0.22)
    )

    new_height = height + step

    if new_height >= OPTIONS_PANEL_HEIGHT:
        new_height = OPTIONS_PANEL_HEIGHT

    options_panel.place_configure(
        height=new_height
    )

    if options_overlay:

        overlay_height = (
            new_height
            + OPTIONS_OVERLAY_PADDING_Y * 2
        )

        options_overlay.place_configure(
            height=overlay_height
        )

    if new_height >= OPTIONS_PANEL_HEIGHT:
        return

    root.after(
        8,
        lambda: animate_open_options_panel(new_height)
    )

def close_options_when_clicking_outside(event):

    if not options_visible:
        return

    if not options_panel:
        return

    clicked_widget = event.widget

    if clicked_widget == options_panel:
        return

    parent = clicked_widget

    while parent:

        if parent == options_panel:
            return

        parent = parent.master

    animate_close_options_panel()

def animate_close_options_panel(height=OPTIONS_PANEL_HEIGHT):

    global options_panel
    global options_overlay
    global options_visible

    if not options_panel:
        options_visible = False
        return

    step = max(
        6,
        int(height * 0.22)
    )

    new_height = height - step

    if new_height <= 1:
        options_panel.destroy()
        options_panel = None

        if options_overlay:
            options_overlay.destroy()
            options_overlay = None

        options_visible = False
        return

    options_panel.place_configure(
        height=new_height
    )

    if options_overlay:

        overlay_height = (
            new_height
            + OPTIONS_OVERLAY_PADDING_Y * 2
        )

        options_overlay.place_configure(
            height=overlay_height
        )

    root.after(
        8,
        lambda: animate_close_options_panel(new_height)
    )


def close_options_panel():

    animate_close_options_panel()

def build_options_header(parent):

    header = tk.Frame(
        parent,
        bg=CARD_BG
    )

    header.pack(
        fill="x",
        padx=12,
        pady=(10, 8)
    )

    tk.Label(
        header,
        text="OPTIONS",
        font=("Segoe UI", 9, "bold"),
        bg=CARD_BG,
        fg=TEXT
    ).pack(
        side="left"
    )

    tk.Button(
        header,
        text="✕",
        font=("Segoe UI", 9),
        bg=CARD_BG,
        fg=MUTED,
        bd=0,
        activebackground=CARD_BG,
        activeforeground="white",
        cursor="hand2",
        command=close_options_panel
    ).pack(
        side="right"
    )


def build_toggle_options(parent):

    tk.Label(
        parent,
        text="GENERAL",
        font=("Segoe UI", 8, "bold"),
        bg=CARD_BG,
        fg=ACCENT
    ).pack(
        anchor="w",
        padx=12,
        pady=(2, 6)
    )

    boot_sound_var = tk.BooleanVar(
        value=state.boot_sound_enabled
    )

    def toggle_boot_sound():

        state.boot_sound_enabled = (
            boot_sound_var.get()
        )

        save_app_config()

    tk.Checkbutton(
        parent,
        text="Enable startup sound",
        variable=boot_sound_var,
        command=toggle_boot_sound,
        font=("Segoe UI", 9),
        bg=CARD_BG,
        fg=TEXT,
        activebackground=CARD_BG,
        activeforeground="white",
        selectcolor="#1a2a2c"
    ).pack(
        anchor="w",
        padx=12,
        pady=(0, 10)
    )

    def open_debug_log():

        log_path = os.path.join(
            APP_DATA_DIR,
            "split_debug.log"
        )

        if not os.path.exists(log_path):

            with open(log_path, "w", encoding="utf-8") as f:
                f.write("SPLIT debug log\n")

        os.startfile(log_path)

    debug_log_button = tk.Button(
        parent,
        text="Open debug_log file",
        font=("Segoe UI", 8),
        bg=OPTIONS_BUTTON_BG,
        fg=TEXT,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=open_debug_log
    )

    debug_log_button.pack(
        anchor="w",
        padx=12,
        pady=(0, 10)
    )

    def open_app_data_folder():

        os.startfile(APP_DATA_DIR)

    app_data_button = tk.Button(
        parent,
        text="Open AppData folder",
        font=("Segoe UI", 8),
        bg=OPTIONS_BUTTON_BG,
        fg=TEXT,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=open_app_data_folder
    )

    app_data_button.pack(
        anchor="w",
        padx=12,
        pady=(0, 10)
    )

    particles_var = tk.BooleanVar(
        value=title_particles_enabled
    )

    def toggle_title_particles():

        global title_particles_enabled

        title_particles_enabled = (
            particles_var.get()
        )

        save_app_config()

    tk.Checkbutton(
        parent,
        text="Enable title particles",
        variable=particles_var,
        command=toggle_title_particles,
        font=("Segoe UI", 9),
        bg=CARD_BG,
        fg=TEXT,
        activebackground=CARD_BG,
        activeforeground="white",
        selectcolor="#1a2a2c"
    ).pack(
        anchor="w",
        padx=12,
        pady=(0, 10)
    )

    def change_game_folder():

        path = filedialog.askdirectory(
            title="Select Deadlock folder"
        )

        if not path:
            return

        cfg_path = os.path.join(
            path,
            "game",
            "citadel",
            "cfg"
        )

        if not os.path.exists(cfg_path):

            ask_confirm_window(
                "Invalid folder",
                "This folder does not look like the main Deadlock folder.\n\n"
                "Please select:\n"
                "Steam\\steamapps\\common\\Deadlock",
                confirm_text="OK",
                danger=True
            )

            return

        state.deadlock_path = path

        save_app_config()
        write_cfg()
        ensure_autoexec()

        show_notification(
            "✅ Game folder updated",
            "success"
        )

        reset_status_after_delay()

    change_folder_button = tk.Button(
        parent,
        text="Change game folder",
        font=("Segoe UI", 8),
        bg=OPTIONS_BUTTON_BG,
        fg=TEXT,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=change_game_folder
    )

    change_folder_button.pack(
        anchor="w",
        padx=12,
        pady=(0, 10)
    )

def create_hotkey_row(
    parent,
    label_text,
    value,
    index,
    shortcut_buttons,
    start_capture,
    add_button_hover
):

    row = tk.Frame(
        parent,
        bg=CARD_BG
    )

    row.pack(
        fill="x",
        padx=12,
        pady=1
    )

    tk.Label(
        row,
        text=label_text,
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED,
        width=9,
        anchor="w"
    ).pack(
        side="left"
    )

    btn = tk.Button(
        row,
        text=value,
        font=("Segoe UI", 8),
        bg=OPTIONS_BUTTON_BG,
        fg=TEXT,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=lambda idx=index: start_capture(idx)
    )

    btn.pack(
        side="right",
        fill="x",
        expand=True
    )

    shortcut_buttons.append(btn)

    add_button_hover(btn)

def build_hotkey_options(parent):

    tk.Label(
        parent,
        text="HOTKEYS",
        font=("Segoe UI", 8, "bold"),
        bg=CARD_BG,
        fg=ACCENT
    ).pack(
        anchor="w",
        padx=12,
        pady=(10, 6)
    )

    shortcut_buttons = []

    waiting_for_key = {
        "index": None
    }

    def add_button_hover(button):

        def on_enter(event):

            button.configure(
                bg=OPTIONS_BUTTON_HOVER_BG,
                fg=TEXT,
                activebackground=OPTIONS_BUTTON_HOVER_ACTIVE
            )

        def on_leave(event):

            button.configure(
                bg=OPTIONS_BUTTON_BG,
                fg=TEXT,
                activebackground=ACCENT
            )

        button.bind("<Enter>", on_enter)
        button.bind("<Leave>", on_leave)

    def get_hotkey_by_index(idx):

        if idx == 0:
            return state.preset_cycle_hotkey

        if idx == 1:
            return state.undo_hotkey

        if idx == 2:
            return state.redo_hotkey

        if idx == 3:
            return state.favorite_mode_hotkey

    def normalize_hotkey(event):

        parts = []

        ctrl_pressed = keyboard.is_pressed("ctrl")
        shift_pressed = keyboard.is_pressed("shift")
        alt_pressed = keyboard.is_pressed("alt")

        if ctrl_pressed:
            parts.append("ctrl")

        if shift_pressed:
            parts.append("shift")

        if alt_pressed:
            parts.append("alt")

        key = event.keysym.lower()

        ignored = [
            "control_l",
            "control_r",
            "shift_l",
            "shift_r",
            "alt_l",
            "alt_r"
        ]

        if key in ignored:
            return None

        if key.startswith("kp_"):
            key = key.replace("kp_", "")

        if key in ["alt", "control", "shift"]:
            return None

        parts.append(key)

        return "+".join(parts)

    def is_reserved_hotkey(hotkey):

        return hotkey in RESERVED_HOTKEYS

    def start_capture(idx):

        waiting_for_key["index"] = idx

        shortcut_buttons[idx].configure(
            text="Press shortcut...",
            bg=ACCENT,
            fg="white"
        )

        root.focus_set()

    def on_key_press(event):

        idx = waiting_for_key["index"]

        if idx is None:
            return

        hotkey = normalize_hotkey(event)

        if not hotkey:
            return

        if hotkey == "escape":

            shortcut_buttons[idx].configure(
                text=get_hotkey_by_index(idx),
                bg=OPTIONS_BUTTON_BG,
                fg=TEXT
            )

            waiting_for_key["index"] = None
            return

        if is_reserved_hotkey(hotkey):

            show_notification(
                "⚠️ Shortcut reserved",
                "warning"
            )

            reset_status_after_delay()

            shortcut_buttons[idx].configure(
                text=get_hotkey_by_index(idx),
                bg=OPTIONS_BUTTON_BG,
                fg=TEXT
            )

            waiting_for_key["index"] = None
            return

        used_hotkeys = [
            state.preset_cycle_hotkey,
            state.undo_hotkey,
            state.redo_hotkey,
            state.favorite_mode_hotkey,
        ]

        current_hotkey = get_hotkey_by_index(idx)

        used_hotkeys = [
            hk for hk in used_hotkeys
            if hk != current_hotkey
        ]

        if hotkey in used_hotkeys:

            show_notification(
                "⚠️ Shortcut already used",
                "warning"
            )

            reset_status_after_delay()

            shortcut_buttons[idx].configure(
                text=current_hotkey,
                bg=OPTIONS_BUTTON_BG,
                fg=TEXT
            )

            waiting_for_key["index"] = None
            return

        if idx == 0:
            state.preset_cycle_hotkey = hotkey

        elif idx == 1:
            state.undo_hotkey = hotkey

        elif idx == 2:
            state.redo_hotkey = hotkey

        elif idx == 3:
            state.favorite_mode_hotkey = hotkey

        shortcut_buttons[idx].configure(
            text=hotkey,
            bg=OPTIONS_BUTTON_BG,
            fg=TEXT
        )

        save_app_config()
        setup_hotkeys()
        refresh_how_to_use_text()

        show_notification(
            "✅ Shortcut updated",
            "success"
        )

        reset_status_after_delay()

        waiting_for_key["index"] = None

    root.bind(
        "<KeyPress>",
        on_key_press
    )

    shortcut_data = [
        ("Preset", state.preset_cycle_hotkey),
        ("Undo", state.undo_hotkey),
        ("Redo", state.redo_hotkey),
        ("Favorite", state.favorite_mode_hotkey),
    ]

    for i, (label_text, value) in enumerate(shortcut_data):

        create_hotkey_row(
            parent,
            label_text,
            value,
            i,
            shortcut_buttons,
            start_capture,
            add_button_hover
        )

    tk.Label(
        parent,
        text="Reserved: H • U/I/O/J/K/L/N/M • F1-F8 • Alt+F1-F8",
        font=("Segoe UI", 7),
        bg=CARD_BG,
        fg=MUTED,
        wraplength=260,
        justify="left"
    ).pack(
        pady=(35, 12)
    )

def toggle_options_panel(anchor_button):

    global options_panel
    global options_visible

    if options_visible:

        close_options_panel()
        return
        
    global options_overlay

    options_overlay = tk.Frame(
        root,
        bg=OPTIONS_OVERLAY_BG,
        highlightbackground=OPTIONS_OVERLAY_BORDER,
        highlightthickness=1,
        bd=0
    )

    options_panel = tk.Frame(
        root,
        bg=CARD_BG,
        highlightbackground=OPTIONS_PANEL_BORDER,
        highlightthickness=1,
        bd=0
    )

    root_x = root.winfo_rootx()
    root_y = root.winfo_rooty()

    button_x = anchor_button.winfo_rootx() - root_x
    button_y = anchor_button.winfo_rooty() - root_y

    panel_x = (
        button_x
        - OPTIONS_PANEL_WIDTH
        + anchor_button.winfo_width()
    )

    panel_y = (
        button_y
        + anchor_button.winfo_height()
        + 0
    )

    overlay_x = (
        panel_x
        - OPTIONS_OVERLAY_PADDING_X
    )

    overlay_y = (
        panel_y
        - 12
        - OPTIONS_OVERLAY_PADDING_Y
    )

    options_overlay.place(
        x=overlay_x,
        y=overlay_y,
        width=(
            OPTIONS_PANEL_WIDTH
            + OPTIONS_OVERLAY_PADDING_X * 2
        ),
        height=1
    )

    options_overlay.lift()

    options_panel.place(
        x=panel_x,
        y=panel_y - 12,
        width=OPTIONS_PANEL_WIDTH,
        height=1
    )

    options_panel.lift()

    build_options_header(options_panel)

    build_toggle_options(options_panel)

    build_hotkey_options(options_panel)

    options_visible = True

    root.bind(
        "<Button-1>",
        close_options_when_clicking_outside,
        add="+"
    )

    animate_open_options_panel()
            
def export_preset():
    path = filedialog.asksaveasfilename(
        defaultextension=".json",
        filetypes=[(f"{APP_NAME} preset", "*.json")],
        title="Export preset"
    )

    if not path:
        return

    data = {
        "app": APP_NAME,
        "version": APP_VERSION,
        "slots": state.slots,
        "slot_names": state.slot_names,
        "slot_colors": state.slot_colors,        
    }

    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=4)

    show_notification(
        "✅ Preset exported",
        "success"
    )

    reset_status_after_delay()


def import_preset():

    path = filedialog.askopenfilename(
        filetypes=[(f"{APP_NAME} preset", "*.json")],
        title="Import preset"
    )

    if not path:
        return
    
    backup_slots_json()

    create_undo_snapshot() 

    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)

    slots = data.get("slots", [])
    names = data.get("slot_names", [])
    colors = data.get("slot_colors", [])    

    for i in range(NUM_SLOTS):
        state.slots[i] = slots[i] if i < len(slots) else None
        state.slot_names[i] = names[i] if i < len(names) else f"Slot {i+1}"
        state.slot_colors[i] = colors[i] if i < len(colors) else None

        if state.slots[i] is None:
            state.slot_colors[i] = None

        state.slot_times[i] = None               


    refresh_slot_display()

    write_cfg()
    save_slots_to_json()

    show_notification(
        "✅ Preset imported",
        "success"
    )

    reset_status_after_delay()            

def create_rounded_button(parent, text, bg_color, fg_color, command, danger_button=False):

    canvas = tk.Canvas(
        parent,
        width=150,
        height=44,
        bg=DARK_BG,
        highlightthickness=0,
        bd=0,
        cursor="hand2"
    )

    normal_border = "#3b4a4d"
    hover_border = "#62ffd9" if not danger_button else "#ff6b7a"
    hover_bg = "#59e0d5" if not danger_button else "#c74358"

    button_bg = bg_color

    rect = canvas.create_round_rectangle(
        2,
        2,
        148,
        42,
        radius=12,
        fill=button_bg,
        outline=normal_border,
        width=1
    )

    label = canvas.create_text(
        75,
        22,
        text=text,
        fill=fg_color,
        font=("Segoe UI", 9, "bold")
    )

    def on_enter(event):

        canvas.itemconfigure(
            rect,
            fill=hover_bg,
            outline=hover_border,
            width=2
        )

        canvas.itemconfigure(
            label,
            fill="#06100e" if text in ["Cancel", "Browse"] else fg_color
        )

    def on_leave(event):

        canvas.itemconfigure(
            rect,
            fill=button_bg,
            outline=normal_border,
            width=1
        )

        canvas.itemconfigure(
            label,
            fill=fg_color
        )

    canvas.bind("<Enter>", on_enter)
    canvas.bind("<Leave>", on_leave)
    canvas.bind("<Button-1>", lambda event: command())

    canvas.tag_bind(rect, "<Button-1>", lambda event: command())
    canvas.tag_bind(label, "<Button-1>", lambda event: command())

    return canvas

def ask_text_window(title, message, initial_value=""):

    result = {
        "value": None
    }

    win = tk.Toplevel(root)
    win.attributes("-alpha", 0.0)
    apply_window_icon(win)
    win.withdraw()
    win.title(title)
    win.configure(bg=DARK_BG)
    win.configure(
        highlightbackground=OPTIONS_PANEL_BORDER,
        highlightthickness=1
    )    
    win.resizable(False, False)
    win.geometry("360x190")
    win.update_idletasks()

    root_x = root.winfo_x()
    root_y = root.winfo_y()

    root_width = root.winfo_width()
    root_height = root.winfo_height()

    win_width = 360
    win_height = 190

    pos_x = root_x + (root_width // 2) - (win_width // 2)
    pos_y = root_y + (root_height // 2) - (win_height // 2)

    win.geometry(
        f"{win_width}x{win_height}+{pos_x}+{pos_y}"
    )    
    win.deiconify()
    win.attributes("-alpha", 1.0)
    win.transient(root)
    win.grab_set()

    tk.Label(
        win,
        text=title,
        font=("Segoe UI", 11, "bold"),
        bg=DARK_BG,
        fg=TEXT
    ).pack(
        anchor="w",
        padx=18,
        pady=(16, 4)
    )

    tk.Label(
        win,
        text=message,
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED
    ).pack(
        anchor="w",
        padx=18,
        pady=(0, 8)
    )

    entry_var = tk.StringVar(
        value=initial_value
    )

    entry = tk.Entry(
        win,
        textvariable=entry_var,
        font=("Segoe UI", 10),
        bg=CARD_BG,
        fg=TEXT,
        insertbackground=TEXT,
        bd=0
    )

    entry.pack(
        fill="x",
        padx=18,
        pady=(0, 14),
        ipady=6
    )

    buttons = tk.Frame(
        win,
        bg=DARK_BG
    )

    buttons.pack(
        fill="x",
        padx=18
    )

    def cancel():

        result["value"] = None
        win.destroy()

    def confirm():

        value = entry_var.get().strip()

        if not value:
            return

        result["value"] = value
        win.destroy()

    cancel_button = create_rounded_button(
        buttons,
        "Cancel",
        DARK_BG,
        MUTED,
        cancel
    )

    cancel_button.pack(
        side="right",
        padx=(8, 0)
    )

    confirm_button = create_rounded_button(
        buttons,
        "Confirm",
        ACCENT,
        "#06100e",
        confirm
    )

    confirm_button.pack(
        side="right"
    )

    entry.focus_set()
    entry.select_range(0, "end")

    win.bind(
        "<Return>",
        lambda event: confirm()
    )

    win.bind(
        "<Escape>",
        lambda event: cancel()
    )

    root.wait_window(win)

    return result["value"]

def _create_round_rectangle(self, x1, y1, x2, y2, radius=12, **kwargs):

    points = [
        x1 + radius, y1,
        x2 - radius, y1,
        x2, y1,
        x2, y1 + radius,
        x2, y2 - radius,
        x2, y2,
        x2 - radius, y2,
        x1 + radius, y2,
        x1, y2,
        x1, y2 - radius,
        x1, y1 + radius,
        x1, y1
    ]

    return self.create_polygon(
        points,
        smooth=True,
        **kwargs
    )


tk.Canvas.create_round_rectangle = _create_round_rectangle

def ask_confirm_window(
    title,
    message,
    confirm_text="Confirm",
    danger=False,
    cancel_text="Cancel",
    center_on_screen=False
):

    result = {
        "value": False
    }

    win = tk.Toplevel(root)
    win.attributes("-alpha", 0.0)
    apply_window_icon(win)
    win.withdraw()
    win.title(title)
    win.configure(bg=DARK_BG)
    win.resizable(False, False)
    win.transient(root)
    win.grab_set()

    if danger and "Reset" in title:

        win_width = 430
        win_height = 300

    elif title == "Game Not Found":

        win_width = 520
        win_height = 300

    elif title == "Invalid Folder":

        win_width = 520
        win_height = 300

    elif title == "Deadlock Folder Found":

        win_width = 520
        win_height = 240

    elif title == "SPLIT Already Running":

        win_width = 390
        win_height = 165

    else:

        win_width = 390
        win_height = 165

    root.update_idletasks()

    root.update_idletasks()

    root_x = root.winfo_x()
    root_y = root.winfo_y()

    root_width = root.winfo_width()
    root_height = root.winfo_height()

    if center_on_screen or root_width <= 1 or root_height <= 1:

        screen_width = root.winfo_screenwidth()
        screen_height = root.winfo_screenheight()

        pos_x = (screen_width // 2) - (win_width // 2)
        pos_y = (screen_height // 2) - (win_height // 2)

    else:

        pos_x = root_x + (root_width // 2) - (win_width // 2)
        pos_y = root_y + (root_height // 2) - (win_height // 2)

    win.geometry(
        f"{win_width}x{win_height}+{pos_x}+{pos_y}"
    )

    win.deiconify()
    win.attributes("-alpha", 1.0)

    tk.Label(
        win,
        text=title,
        font=("Segoe UI", 11, "bold"),
        bg=DARK_BG,
        fg="#d98c8c" if danger else TEXT
    ).pack(
        anchor="w",
        padx=18,
        pady=(18, 8)
    )

    tk.Label(
        win,
        text=message,
        font=("Segoe UI", 9),
        bg=DARK_BG,
        fg=MUTED,
        wraplength=380,
        justify="left"
    ).pack(
        anchor="w",
        padx=18,
        pady=(0, 18)
    )

    buttons = tk.Frame(
        win,
        bg=DARK_BG
    )

    buttons.pack(
        fill="x",
        padx=18,
        pady=(6, 0)
    )

    def cancel():

        result["value"] = False
        win.destroy()

    def confirm():

        result["value"] = True
        win.destroy()

    cancel_button = create_rounded_button(
        buttons,
        cancel_text,
        DARK_BG,
        MUTED,
        cancel
    )

    cancel_button.pack(
        side="right",
        padx=(8, 0)
    )

    confirm_button = create_rounded_button(
        buttons,
        confirm_text,
        "#a83246" if danger else ACCENT,
        "white" if danger else "#06100e",
        confirm,
        danger_button=danger
    )

    confirm_button.pack(
        side="right"
    )

    win.bind(
        "<Escape>",
        lambda event: cancel()
    )

    win.bind(
        "<Return>",
        lambda event: confirm()
    )

    root.wait_window(win)

    return result["value"]

def rename_slot(idx: int):

    new_name = ask_text_window(
        "Rename Slot",
        f"New name for slot {idx + 1}:",
        state.slot_names[idx]
    )

    if not new_name:
        return

    state.slot_names[idx] = new_name

    label = new_name
    label = label[:48] + "…" if len(label) > 48 else label

    state.slot_vars[idx].set(label)

    root.after(
        0,
        lambda idx=idx: flash_slot_safe(idx, "rename")
    )

    show_notification(
        f"✏️ Slot {idx+1} renamed",
        "success"
    )

    reset_status_after_delay()

    save_slots_to_json()            

def open_favorite_save_window(source_idx: int):

    if state.slots[source_idx] is None:

        show_notification(
            f"⚠️ Slot {source_idx+1} empty",
            "warning"
        )

        reset_status_after_delay()

        return

    win = tk.Toplevel(root)
    apply_window_icon(win)
    win.title("Save Favorite")
    win.configure(bg=DARK_BG)
    win.resizable(False, False)
    win.withdraw()

    win_width = 400
    win_height = 380

    root.update_idletasks()

    root_x = root.winfo_x()
    root_y = root.winfo_y()

    root_width = root.winfo_width()
    root_height = root.winfo_height()

    pos_x = root_x + (root_width // 2) - (win_width // 2)
    pos_y = root_y + (root_height // 2) - (win_height // 2)

    win.geometry(
        f"{win_width}x{win_height}+{pos_x}+{pos_y}"
    )

    win.deiconify()

    selected_favorite = tk.IntVar(value=source_idx)
    favorite_name = tk.StringVar(value=state.slot_names[source_idx])

    tk.Label(
        win,
        text=f"Save Slot {source_idx+1} to favorites",
        font=("Segoe UI", 11, "bold"),
        bg=DARK_BG,
        fg=TEXT
    ).pack(anchor="w", padx=16, pady=(14, 8))

    tk.Label(
        win,
        text="Choose favorite slot:",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED
    ).pack(anchor="w", padx=16, pady=(0, 6))

    favorite_grid = tk.Frame(
        win,
        bg=DARK_BG
    )
    favorite_grid.pack(fill="x", padx=16, pady=(0, 10))

    favorite_buttons = []

    def select_favorite(fav_idx):

        selected_favorite.set(fav_idx)

        for i, button in enumerate(favorite_buttons):

            active = i == fav_idx

            button.configure(
                bg=ACCENT if active else OPTIONS_BUTTON_BG,
                fg="#06100e" if active else TEXT
            )

    for fav_idx in range(NUM_SLOTS):

        btn = tk.Button(
            favorite_grid,
            text=f"Favorite {fav_idx + 1}",
            font=("Segoe UI", 8, "bold"),
            bg=ACCENT if fav_idx == selected_favorite.get() else OPTIONS_BUTTON_BG,
            fg="#06100e" if fav_idx == selected_favorite.get() else TEXT,
            bd=0,
            padx=12,
            pady=6,
            activebackground=ACCENT,
            activeforeground="#06100e",
            cursor="hand2",
            command=lambda idx=fav_idx: select_favorite(idx)
        )

        btn.grid(
            row=fav_idx // 2,
            column=fav_idx % 2,
            sticky="ew",
            padx=4,
            pady=4
        )

        favorite_buttons.append(btn)

    favorite_grid.grid_columnconfigure(
        0,
        weight=1
    )

    favorite_grid.grid_columnconfigure(
        1,
        weight=1
    )

    tk.Label(
        win,
        text="Favorite name:",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED
    ).pack(anchor="w", padx=16, pady=(2, 4))

    name_entry = tk.Entry(
        win,
        textvariable=favorite_name,
        font=("Segoe UI", 9),
        bg=CARD_BG,
        fg=TEXT,
        insertbackground=TEXT,
        bd=0
    )
    name_entry.pack(
        fill="x",
        padx=16,
        pady=(0, 12),
        ipady=6
    )
    name_entry.focus_set()

    buttons = tk.Frame(
        win,
        bg=DARK_BG
    )
    buttons.pack(fill="x", padx=16, pady=(4, 0))

    def confirm():

        fav_idx = selected_favorite.get()

        name = favorite_name.get().strip()

        if not name:
            name = f"Favorite {fav_idx+1}"

        state.favorite_slots[fav_idx] = state.slots[source_idx]
        state.favorite_names[fav_idx] = name

        save_slots_to_json()

        show_notification(
            f"⭐ Slot {source_idx+1} saved to Favorite {fav_idx+1}",
            "success"
        )

        reset_status_after_delay()

        win.destroy()

    tk.Button(
        buttons,
        text="Cancel",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED,
        bd=0,
        padx=10,
        pady=5,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=win.destroy
    ).pack(side="right", padx=(6, 0))

    tk.Button(
        buttons,
        text="Save Favorite",
        font=("Segoe UI", 8, "bold"),
        bg=ACCENT,
        fg="white",
        bd=0,
        padx=10,
        pady=5,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=confirm
    ).pack(side="right")


# Console log watcher 

def tail_log():

    while state.running:

        try:

            if not os.path.exists(state.console_log):

                time.sleep(1)
                continue

            with open(state.console_log, "r", encoding="utf-8", errors="ignore") as f:

                # SKIP TO THE END OF THE FILE
                f.seek(0, os.SEEK_END)

                while state.running:

                    line = f.readline()

                    if not line:
                        time.sleep(0.01)
                        continue

                    m = SETPOS_RE.search(line)

                    if m and state.pending_save_slot is not None:

                        slot_idx = state.pending_save_slot
                        state.pending_save_slot = None

                        coords = m.group(0)

                        save_slot(slot_idx, coords)

        except Exception as e:

            debug_error("tail_log", e)

            time.sleep(1)

def reset_status_after_delay(delay_ms=2000):
    def reset():
        if is_deadlock_running():
            state.status_msg.set(GAME_RUNNING_STATUS)
        else:
            state.status_msg.set(DEFAULT_STATUS)

    root.after(delay_ms, reset)

def save_slot(idx: int, coords: str):

    create_undo_snapshot()

    if state.favorite_mode:

        state.favorite_slots[idx] = coords

        if state.favorite_names[idx] == f"Favorite {idx+1}":
            state.favorite_names[idx] = f"Favorite Save {idx+1}"

        label = state.favorite_names[idx]

    else:

        state.slots[idx] = coords
        state.slot_times[idx] = time.time()

        if state.slot_names[idx] == f"Slot {idx+1}":
            state.slot_names[idx] = f"Save {idx+1}"

        label = state.slot_names[idx]

    label = label[:48] + "…" if len(label) > 48 else label

    saved_time = format_saved_time(
        state.slot_times[idx]
    )

    state.slot_vars[idx].set(
        f"{label}  ·  {saved_time}"
    )

    root.after(
        0,
        lambda idx=idx: flash_slot_safe(idx, "save")
    )

    if state.favorite_mode:

        show_notification(
            f"⭐ Favorite {idx+1} saved",
            "success"
        )

    else:

        show_notification(
            f"✅ Slot {idx+1} saved",
            "success"
        )

    reset_status_after_delay()

    write_cfg()

    save_slots_to_json()


def load_slot(idx: int):

    active_slots = state.favorite_slots if state.favorite_mode else state.slots

    pos = active_slots[idx]

    if not pos:

        if state.favorite_mode:

            show_notification(
                f"⚠️ Favorite {idx+1} empty",
                "warning"
            )

        else:

            show_notification(
                f"⚠️ Slot {idx+1} empty",
                "warning"
            )

        reset_status_after_delay()

        return

    keyboard.send(LOAD_KEYS[idx])
    
    debug_log(f"Load hotkey pressed: slot {idx + 1}")    

    root.after(
        0,
        lambda idx=idx: flash_slot_safe(idx, "load")
    )


    if state.favorite_mode:

        show_notification(
            f"⭐ Loaded favorite {idx+1}",
            "load"
        )

    else:

        show_notification(
            f"📍 Loaded slot {idx+1}",
            "load"
        )

    reset_status_after_delay()


def trigger_reload():
    """Append exec savestate to autoexec so next bind reloads it.
    Actually we rely on the in-game bind reloading cfg on demand."""
    pass   # binds are persistent once loaded; reload happens on next exec


def load_app_config():
    if not os.path.exists(APP_CONFIG_FILE):
        return

    with open(APP_CONFIG_FILE, "r", encoding="utf-8") as f:
        config = json.load(f)

    state.preset_cycle_hotkey = config.get(
        "preset_cycle_hotkey",
        DEFAULT_PRESET_CYCLE_KEY
    )

    state.undo_hotkey = config.get(
        "undo_hotkey",
        "f9"
    )

    state.redo_hotkey = config.get(
        "redo_hotkey",
        "f10"
    )

    state.favorite_mode_hotkey = config.get(
        "favorite_mode_hotkey",
        DEFAULT_FAVORITE_MODE_HOTKEY
    )

    state.show_favorite_warning = config.get(
        "show_favorite_warning",
        True
    )

    state.boot_sound_enabled = config.get(
        "boot_sound_enabled",
        True
    )

    global title_particles_enabled

    title_particles_enabled = config.get(
        "title_particles_enabled",
        True
    )

    hotkeys = config.get("preset_hotkeys")

    if isinstance(hotkeys, list):
        for i in range(4):
            if i < len(hotkeys) and hotkeys[i]:
                state.preset_hotkeys[i] = hotkeys[i]


def save_app_config():
    config = {
        "deadlock_path": state.deadlock_path,
        "preset_cycle_hotkey": state.preset_cycle_hotkey,
        "preset_hotkeys": state.preset_hotkeys,
        "undo_hotkey": state.undo_hotkey,
        "redo_hotkey": state.redo_hotkey,
        "favorite_mode_hotkey": state.favorite_mode_hotkey,
        "show_favorite_warning": state.show_favorite_warning,
        "boot_sound_enabled": state.boot_sound_enabled,
        "title_particles_enabled": title_particles_enabled,
    }

    with open(APP_CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(config, f, indent=4)

# Hotkeys
registered_hotkeys = []

def debug_log(message):

    try:

        with open(
            os.path.join(APP_DATA_DIR, "split_debug.log"),
            "a",
            encoding="utf-8"
        ) as f:

            timestamp = time.strftime("%H:%M:%S")

            f.write(f"[{timestamp}] {message}\n")

    except Exception:
        pass
        
def debug_error(context, error):

    try:

        with open(
            os.path.join(APP_DATA_DIR, "split_debug.log"),
            "a",
            encoding="utf-8"
        ) as f:

            timestamp = time.strftime("%H:%M:%S")

            f.write(
                f"[{timestamp}] ERROR in {context}: {error}\n"
            )

    except Exception:
        pass        

def get_active_window_process_name():

    try:

        user32 = ctypes.windll.user32

        hwnd = user32.GetForegroundWindow()

        if not hwnd:
            return ""

        pid = ctypes.c_ulong()

        user32.GetWindowThreadProcessId(
            hwnd,
            ctypes.byref(pid)
        )

        process = psutil.Process(
            pid.value
        )

        return process.name().lower()

    except Exception:
        return ""


def is_split_or_deadlock_active():

    active_process = get_active_window_process_name()

    return active_process in [
        "deadlock.exe",
        os.path.basename(sys.executable).lower()
    ]

def cycle_preset():

    if not is_split_or_deadlock_active():
        return

    next_preset = (
        state.current_preset + 1
    ) % 4

    switch_preset(next_preset)

def show_favorite_mode_warning():

    if not state.show_favorite_warning:
        return

    result = {
        "dont_show_again": False
    }

    win = tk.Toplevel(root)
    win.attributes("-alpha", 0.0)
    apply_window_icon(win)
    win.withdraw()
    win.title("Favorite Mode")
    win.configure(bg=DARK_BG)
    win.resizable(False, False)

    win_width = 430
    win_height = 210

    root.update_idletasks()

    root_x = root.winfo_x()
    root_y = root.winfo_y()
    root_width = root.winfo_width()
    root_height = root.winfo_height()

    pos_x = root_x + (root_width // 2) - (win_width // 2)
    pos_y = root_y + (root_height // 2) - (win_height // 2)

    win.geometry(
        f"{win_width}x{win_height}+{pos_x}+{pos_y}"
    )

    win.deiconify()
    win.attributes("-alpha", 1.0)
    win.transient(root)
    win.grab_set()

    tk.Label(
        win,
        text="Favorite Mode",
        font=("Segoe UI", 11, "bold"),
        bg=DARK_BG,
        fg=TEXT
    ).pack(
        anchor="w",
        padx=18,
        pady=(18, 8)
    )

    tk.Label(
        win,
        text=(
            "When Favorite Mode is enabled, saving a slot will overwrite "
            "the favorite slot, not the normal preset slot."
        ),
        font=("Segoe UI", 9),
        bg=DARK_BG,
        fg=MUTED,
        wraplength=390,
        justify="left"
    ).pack(
        anchor="w",
        padx=18,
        pady=(0, 14)
    )

    dont_show_var = tk.BooleanVar(
        value=False
    )

    tk.Checkbutton(
        win,
        text="Don't show this message again",
        variable=dont_show_var,
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED,
        activebackground=DARK_BG,
        activeforeground=TEXT,
        selectcolor=CARD_BG
    ).pack(
        anchor="w",
        padx=18,
        pady=(0, 14)
    )

    def confirm():

        result["dont_show_again"] = dont_show_var.get()

        if result["dont_show_again"]:

            state.show_favorite_warning = False
            save_app_config()

        win.destroy()

    buttons = tk.Frame(
        win,
        bg=DARK_BG
    )

    buttons.pack(
        fill="x",
        padx=18
    )

    ok_button = create_rounded_button(
        buttons,
        "OK",
        ACCENT,
        "#06100e",
        confirm
    )

    ok_button.pack(
        side="right"
    )

    win.bind(
        "<Return>",
        lambda event: confirm()
    )

    win.bind(
        "<Escape>",
        lambda event: confirm()
    )

    root.wait_window(win)

def toggle_favorite_mode():

    will_enable_favorites = not state.favorite_mode

    if will_enable_favorites:
        show_favorite_mode_warning()

    state.favorite_mode = not state.favorite_mode

    refresh_slot_display()

    write_cfg()

    if favorite_mode_btn:

        favorite_mode_btn.configure(
            text="★" if state.favorite_mode else "☆",
            fg="#ffd166" if state.favorite_mode else MUTED
        )

    if state.favorite_mode:

        show_notification(
            "⭐ Favorite mode enabled",
            "success"
        )

    else:

        show_notification(
            "☆ Favorite mode disabled",
            "warning"
        )

    reset_status_after_delay()    

def setup_hotkeys():

    global registered_hotkeys

    debug_log("Rebuilding hotkeys...")

    for hotkey in registered_hotkeys:

        try:
            keyboard.remove_hotkey(hotkey)

        except Exception as e:
            debug_error("remove_hotkey", e)

    registered_hotkeys.clear()

    for i in range(NUM_SLOTS):

        slot = i

        hk1 = keyboard.add_hotkey(
            f"f{i+1}",
            lambda s=slot: on_save_hotkey(s),
            suppress=True,
        )

        registered_hotkeys.append(hk1)

        hk2 = keyboard.add_hotkey(
            f"alt+f{i+1}",
            lambda s=slot: on_load_hotkey(s),
            suppress=True,
        )

        registered_hotkeys.append(hk2)

    hk_preset = keyboard.add_hotkey(
        state.preset_cycle_hotkey,
        lambda: root.after(
            0,
            cycle_preset
        ),
        suppress=False
    )

    registered_hotkeys.append(hk_preset)

    hk_favorite = keyboard.add_hotkey(
        state.favorite_mode_hotkey,
        lambda: root.after(
            0,
            toggle_favorite_mode
        ),
        suppress=False
    )

    registered_hotkeys.append(hk_favorite)    

    hk_undo = keyboard.add_hotkey(
        state.undo_hotkey,
        lambda: root.after(
            0,
            undo_last_action
        ),
        suppress=False
    )

    registered_hotkeys.append(hk_undo)

    hk_redo = keyboard.add_hotkey(
        state.redo_hotkey,
        lambda: root.after(
            0,
            redo_last_action
        ),
        suppress=False
    )

    registered_hotkeys.append(hk_redo)
    
    debug_log("Hotkeys registered successfully")    

def on_save_hotkey(slot: int):

    if not is_deadlock_running():

        show_notification(
            "⚠️ Deadlock is not running",
            "warning"
        )

        reset_status_after_delay()
        return
    
    state.pending_save_slot = slot
    
    debug_log(f"Save hotkey pressed: slot {slot + 1}")   
    
    keyboard.send(SAVE_KEY)
    
    state.status_msg.set(f"⏳ Saving slot {slot+1}...")
    reset_status_after_delay()
    
    threading.Timer(6.0, clear_pending_save).start()
    
def clear_pending_save():

    if state.pending_save_slot is not None:

        state.pending_save_slot = None

        if not is_deadlock_running():

            show_notification(
                "⚠️ Deadlock is not running",
                "warning"
            )

        elif not os.path.exists(state.console_log):

            ask_confirm_window(
                "console.log not found",
                f"SPLIT is looking here:\n\n{state.console_log}",
                confirm_text="OK",
                danger=True
            )

        else:

            show_notification(
                "⚠️ No getpos response — check -condebug -consolelog",
                "warning"
            )

        reset_status_after_delay()


def on_load_hotkey(slot: int):
    load_slot(slot)

def is_deadlock_running():
    process_names = [
        "deadlock.exe"
    ]

    for proc in psutil.process_iter(["name"]):
        try:
            name = proc.info["name"]

            if name and name.lower() in process_names:
                return True

        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue

    return False

slot_rows = []

favorite_mode_btn = None

options_panel = None
options_overlay = None
options_visible = False
title_particles_enabled = True

notification_frame = None
last_deadlock_running = False

def center_status_text():

    try:

        status_canvas.update_idletasks()

        bbox = status_canvas.bbox(status_text)

        if not bbox:
            return

        text_width = bbox[2] - bbox[0]

        canvas_width = status_canvas.winfo_width()

        centered_x = (
            canvas_width - text_width
        ) / 2

        status_canvas.coords(
            status_text,
            centered_x,
            31
        )

    except Exception:
        pass

def show_notification(text, mode="success"):

    state.status_msg.set(text)

    center_status_text()

notification_label = None
notification_after_id = None

def flash_slot_safe(idx: int, mode="save"):

    if idx < 0:
        return

    if idx >= len(slot_rows):
        return

    row = slot_rows[idx]

    if not hasattr(row, "flash_slot"):
        return

    try:
        row.flash_slot(mode)

    except Exception:
        pass

# GUI

DARK_BG     = "#0d1718"
CARD_BG     = "#142022"
ACCENT      = "#4fd1c5"
TEXT        = "#d8e6e3"
MUTED       = "#9aa9a6"
OPTIONS_BUTTON_BG = "#1a2a2c"
OPTIONS_BUTTON_HOVER_BG = "#24494b"
OPTIONS_BUTTON_HOVER_ACTIVE = "#2f6f68"
OPTIONS_OVERLAY_BG = "#030909"
OPTIONS_OVERLAY_BORDER = "#2f6f68"
OPTIONS_PANEL_BORDER = "#4fd1c5"
SUCCESS     = "#62ffd9"
WARN        = "#d8a84f"
SLOT_EMPTY  = "#1b292b"
SLOT_FILLED = "#213a3a"
SLOT_TAG_COLORS = [
    None,
    "#4fd1c5",  # cyan
    "#ffd166",  # yellow
    "#d98c8c",  # red
    "#9b8cff",  # purple
    "#62ff8f",  # green
]
FONT_MAIN = ("Segoe UI", 10)
FONT_BOLD = ("Segoe UI", 10, "bold")
FONT_TITLE= ("Segoe UI", 18, "bold")
FONT_MONO = ("Consolas", 9)
FONT_MONO_ITALIC = ("Consolas", 9, "italic")

def apply_button_hover(
    button,
    normal_bg,
    normal_fg,
    hover_bg=ACCENT,
    hover_fg="white"
):

    def on_enter(event):

        button.configure(
            bg=hover_bg,
            fg=hover_fg
        )

    def on_leave(event):

        button.configure(
            bg=normal_bg,
            fg=normal_fg
        )

    button.bind("<Enter>", on_enter)
    button.bind("<Leave>", on_leave)

def sync_status_text(*_):

    status_canvas.itemconfigure(
        status_text,
        text=state.status_msg.get()
    )

    center_status_text()

def animate_status_text(step=0):

    colors = [
        "#62ffd9",
        "#74ffe0",
        "#8effe7",
        "#74ffe0"
    ]

    status_canvas.itemconfigure(
        status_text,
        fill=colors[step % len(colors)]
    )

    root.after(
        900,
        lambda: animate_status_text(step + 1)
    )

def animate_status_scroll():

    if not is_deadlock_running():

        state.status_msg.set(DEFAULT_STATUS)

        root.after(
            500,
            animate_status_scroll
        )

        return

    x, y = status_canvas.coords(status_text)

    x += 1.2

    bbox = status_canvas.bbox(status_text)

    if bbox:

        text_width = bbox[2] - bbox[0]

        if x > 700:

            new_text = random.choice(STATUS_MESSAGES)

            status_canvas.itemconfigure(
                status_text,
                text=new_text
            )

            new_bbox = status_canvas.bbox(status_text)

            if new_bbox:

                new_text_width = new_bbox[2] - new_bbox[0]

            else:

                new_text_width = text_width

            x = -new_text_width

    status_canvas.coords(
        status_text,
        x,
        y
    )

    root.after(
        16,
        animate_status_scroll
    )

def flash_status_bar_on_game_detected(step=0):

    colors = [
        "#1f4a47",
        "#2f6f68",
        "#4fd1c5",
        "#2f6f68",
        "#284347"
    ]

    if step >= len(colors):
        status_canvas.configure(
            highlightbackground="#284347"
        )
        return

    status_canvas.configure(
        highlightbackground=colors[step]
    )

    root.after(
        90,
        lambda: flash_status_bar_on_game_detected(step + 1)
    )

def update_game_status():

    global last_deadlock_running

    deadlock_running = is_deadlock_running()

    if deadlock_running and not last_deadlock_running:

        flash_status_bar_on_game_detected()

        state.status_msg.set(
            GAME_RUNNING_STATUS
        )

    if deadlock_running:

        if state.status_msg.get() == DEFAULT_STATUS:
            state.status_msg.set(GAME_RUNNING_STATUS)

    else:

        if state.status_msg.get() == GAME_RUNNING_STATUS:
            state.status_msg.set(DEFAULT_STATUS)

    last_deadlock_running = deadlock_running

    root.after(
        1500,
        update_game_status
    )

def build_gui(root):

    global refresh_how_to_use_text

    def resource_path(relative_path):

        try:
            base_path = sys._MEIPASS

        except Exception:
            base_path = os.path.abspath(".")

        return os.path.join(base_path, relative_path)
        
        
    root.title(APP_NAME)
    apply_window_icon(root)  
    root.configure(bg=DARK_BG)
    root.resizable(False, False)
    root.geometry("700x950")
     

    # ── Title Bar
    title_frame = tk.Frame(root, bg="#081011", height=42)
    title_frame.pack(fill="x", padx=20, pady=(16, 4))
    title_frame.pack_propagate(False)

    title_canvas = tk.Canvas(
        title_frame,
        bg="#081011",
        height=42,
        highlightthickness=0,
        bd=0
    )

    title_canvas.pack(
        fill="both",
        expand=True
    )

    title_font = ("Bahnschrift SemiBold", 30)
    title_x = 330
    title_y = 13


    title_img = Image.new(
        "RGBA",
        (220, 60),
        (0, 0, 0, 0)
    )

    title_text = APP_NAME

    title_pil_font = ImageFont.truetype(
        resource_path("bahnschrift.ttf"),
        38
    )
    
    title_draw = ImageDraw.Draw(title_img)

    bbox = title_draw.textbbox(
        (0, 0),
        title_text,
        font=title_pil_font
    )

    text_width = bbox[2] - bbox[0]
    text_height = bbox[3] - bbox[1]

    text_x = (220 - text_width) // 2
    text_y = (60 - text_height) // 2 - 2

    glow_img = Image.new(
        "RGBA",
        (220, 60),
        (0, 0, 0, 0)
    )

    glow_draw = ImageDraw.Draw(glow_img)

    glow_draw.text(
        (text_x, text_y),
        title_text,
        font=title_pil_font,
        fill=(98, 255, 217, 170)
    )

    glow_img = glow_img.filter(
        ImageFilter.GaussianBlur(8)
    )

    title_img = Image.alpha_composite(
        title_img,
        glow_img
    )

    mask = Image.new(
        "L",
        (220, 60),
        0
    )

    mask_draw = ImageDraw.Draw(mask)

    mask_draw.text(
        (text_x, text_y),
        title_text,
        font=title_pil_font,
        fill=255
    )

    gradient = Image.new(
        "RGBA",
        (220, 60),
        (0, 0, 0, 0)
    )

    for y in range(60):

        ratio = y / 60

        r = int(120 - ratio * 70)
        g = int(255 - ratio * 90)
        b = int(235 - ratio * 110)

        for x in range(220):

            gradient.putpixel(
                (x, y),
                (r, g, b, 255)
            )

    gradient.putalpha(mask)

    title_img = Image.alpha_composite(
        title_img,
        gradient
    )

    tk_title_img = ImageTk.PhotoImage(title_img)

    title_canvas.create_image(
        title_x,
        title_y,
        image=tk_title_img,
        anchor="center"
    )

    title_canvas.title_image = tk_title_img    
    
        # Left arrow
    title_canvas.create_line(
        135,
        title_y,
        255,
        title_y,
        fill="#1f4a47",
        width=1
    )

    # Left diamond
    title_canvas.create_polygon(
        260,
        title_y - 4,
        264,
        title_y,
        260,
        title_y + 4,
        256,
        title_y,
        fill="#4fd1c5",
        outline=""
    )

    # Right arrow
    title_canvas.create_line(
        405,
        title_y,
        525,
        title_y,
        fill="#1f4a47",
        width=1
    )

    # Right diamond
    title_canvas.create_polygon(
        400,
        title_y - 4,
        404,
        title_y,
        400,
        title_y + 4,
        396,
        title_y,
        fill="#4fd1c5",
        outline=""
    )
    
    particles = []

    particle_colors = [
        "#244844",
        "#2f6f68",
        "#3fa89c",
        "#62ffd9",
        "#3fa89c",
        "#2f6f68"
    ]

    particle_positions = [
        (42, 11),
        (76, 27),
        (121, 16),
        (148, 33),

        (503, 31),
        (552, 13),
        (591, 24),
        (628, 9)
    ]

    for base_x, base_y in particle_positions:

        x = base_x + random.randint(-10, 10)

        y = base_y + random.randint(-6, 6)

        size = random.choice([1, 1, 2])

        particle = title_canvas.create_oval(
            x,
            y,
            x + size,
            y + size,
            fill=random.choice(particle_colors),
            outline=""
        )

        particles.append(particle)

    def animate_title_particles():

        if not title_particles_enabled:

            for particle in particles:

                title_canvas.itemconfigure(
                    particle,
                    state="hidden"
                )

            root.after(
                260,
                animate_title_particles
            )

            return

        for particle in particles:

            title_canvas.itemconfigure(
                particle,
                state="normal"
            )

        for particle in particles:

            if random.random() < 0.22:

                title_canvas.itemconfigure(
                    particle,
                    fill=random.choice(particle_colors)
                )

        root.after(
            260,
            animate_title_particles
        )

    animate_title_particles()
    

    for particle in particles:

        title_canvas.tag_raise(particle)    

    tk.Label(
        title_frame,
        text=f"v{APP_VERSION}",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED
    ).place(x=628, y=16)

    # ── Status bar

    global status_canvas

    status_canvas = tk.Canvas(
        root,
        bg=CARD_BG,
        height=62,
        highlightbackground="#284347",
        highlightthickness=1,
        bd=0
    )
    status_canvas.pack(fill="x", padx=20, pady=(4, 10))

    LOGO_COLOR = (98, 255, 217)


    base_logo_original = Image.open(
        resource_path("deadlock_logo.png")
    ).convert("RGBA")

    alpha = base_logo_original.getchannel("A")

    base_logo = Image.new(
        "RGBA",
        base_logo_original.size,
        (*LOGO_COLOR, 0)
    )
    base_logo.putalpha(alpha)

    logo_img = base_logo.resize(
        (28, 28),
        Image.LANCZOS
    )

    rotation_angle = 0

    status_logo_mask = status_canvas.create_rectangle(
        0,
        0,
        52,
        62,
        fill=CARD_BG,
        outline=""
    )

    status_logo = status_canvas.create_image(
        40,
        31,
        image=None
    )   

    def animate_logo():
        nonlocal rotation_angle

        rotation_angle += 0.25

        rotated = logo_img.rotate(
            rotation_angle,
            resample=Image.BICUBIC,
            expand=True
        )

        canvas_size = 40

        rotated_base = Image.new(
            "RGBA",
            (canvas_size, canvas_size),
            (0, 0, 0, 0)
        )

        x = (canvas_size - rotated.width) // 2
        y = (canvas_size - rotated.height) // 2

        rotated_base.paste(rotated, (x, y), rotated)

        alpha = rotated_base.getchannel("A")

        glow_alpha = alpha.filter(
            ImageFilter.GaussianBlur(4)
        )

        glow_alpha = glow_alpha.point(
            lambda p: int(p * 0.30)
        )

        glow = Image.new(
            "RGBA",
            rotated_base.size,
            (*LOGO_COLOR, 0)
        )
        glow.putalpha(glow_alpha)

        final = Image.alpha_composite(
            glow,
            rotated_base
        )

        tk_img = ImageTk.PhotoImage(final)

        status_canvas.itemconfigure(
            status_logo,
            image=tk_img
        )
        status_canvas.logo_image = tk_img
        status_canvas.tag_raise(status_logo_mask)
        status_canvas.tag_raise(status_logo)

        root.after(16, animate_logo)

    # Main text
    
    global status_text

    status_text = status_canvas.create_text(
        200,
        31,
        text=state.status_msg.get(),
        font=("Segoe UI", 10, "bold"),
        fill="#62ffd9",
        anchor="w"
    )
        
    state.status_msg.trace_add(
        "write",
        sync_status_text
    )
       
    animate_logo()

    animate_status_text()
    
    animate_status_scroll()
    
    update_game_status()
   
    # ── Presets bar
    preset_frame = tk.Frame(
        root,
        bg=DARK_BG
    )
    preset_frame.pack(fill="x", padx=20, pady=(0, 10))

    global preset_buttons
    preset_buttons = []

    def update_preset_buttons():
        for idx, btn in enumerate(preset_buttons):
            active = idx == state.current_preset

            btn.configure(
                text=state.presets[idx]["name"],
                bg=ACCENT if active else CARD_BG,
                fg="#06100e" if active else MUTED
            )

    globals()["update_preset_buttons"] = update_preset_buttons

    tk.Label(
        preset_frame,
        text="PRESETS",
        font=("Segoe UI", 8, "bold"),
        bg=DARK_BG,
        fg=MUTED
    ).pack(side="left", padx=(0, 8))

    for i in range(4):
        btn = tk.Button(
            preset_frame,
            text=state.presets[i]["name"],
            font=("Segoe UI", 8),
            bg=CARD_BG,
            fg=MUTED,
            bd=0,
            padx=10,
            pady=4,
            activebackground=ACCENT,
            activeforeground="white",
            cursor="hand2",
            command=lambda idx=i: switch_preset(idx)
        )
        btn.pack(side="left", padx=(0, 6))
        preset_buttons.append(btn)

    undo_button = tk.Button(
        preset_frame,
        text="⟲",
        font=("Segoe UI", 11, "bold"),
        bg=DARK_BG,
        fg=MUTED,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=undo_last_action
    )

    undo_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        undo_button,
        DARK_BG,
        MUTED
    )
    
    redo_button = tk.Button(
        preset_frame,
        text="⟳",
        font=("Segoe UI", 11, "bold"),
        bg=DARK_BG,
        fg=MUTED,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=redo_last_action
    )

    redo_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        redo_button,
        DARK_BG,
        MUTED
    )    

    options_button = tk.Button(
        preset_frame,
        text="⚙",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2"
    )

    options_button.configure(
        command=lambda: toggle_options_panel(options_button)
    )

    options_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        options_button,
        DARK_BG,
        MUTED
    )

    factory_reset_button = tk.Button(
        preset_frame,
        text="Reset All",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg="#d98c8c",
        bd=0,
        padx=8,
        pady=4,
        activebackground="#a83246",
        activeforeground="white",
        cursor="hand2",
        command=reset_all_data
    )

    factory_reset_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        factory_reset_button,
        CARD_BG,
        "#d98c8c",
        hover_bg="#a83246"
    )

    clear_slots_button = tk.Button(
        preset_frame,
        text="Clear Slots",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=clear_current_preset_slots
    )

    clear_slots_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        clear_slots_button,
        CARD_BG,
        MUTED
    )

    rename_preset_button = tk.Button(
        preset_frame,
        text="Rename",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED,
        bd=0,
        padx=8,
        pady=4,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=rename_current_preset
    )

    rename_preset_button.pack(
        side="right"
    )

    apply_button_hover(
        rename_preset_button,
        CARD_BG,
        MUTED
    )

    update_preset_buttons()

    # ── Slots grid
    slots_frame = tk.Frame(root, bg=DARK_BG)
    global slot_rows
    global favorite_mode_btn

    slot_rows = []
    slots_frame.pack(fill="both", padx=20)

    slots_header = tk.Frame(
        slots_frame,
        bg=DARK_BG
    )
    slots_header.pack(fill="x", pady=(0, 6))

    tk.Label(
        slots_header,
        text="POSITION SLOTS",
        font=("Segoe UI", 8, "bold"),
        bg=DARK_BG,
        fg=MUTED
    ).pack(side="left")

    import_button = tk.Button(
        slots_header,
        text="Import",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED,
        bd=0,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=import_preset
    )

    import_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        import_button,
        DARK_BG,
        MUTED
    )

    export_button = tk.Button(
        slots_header,
        text="Export",
        font=("Segoe UI", 8),
        bg=DARK_BG,
        fg=MUTED,
        bd=0,
        activebackground=ACCENT,
        activeforeground="white",
        cursor="hand2",
        command=export_preset
    )

    export_button.pack(
        side="right",
        padx=(6, 0)
    )

    apply_button_hover(
        export_button,
        DARK_BG,
        MUTED
    )

    favorite_mode_btn = tk.Button(
        slots_header,
        text="★" if state.favorite_mode else "☆",
        font=("Segoe UI", 11),
        bg=DARK_BG,
        fg="#ffd166" if state.favorite_mode else MUTED,
        bd=0,
        activebackground=ACCENT,
        activeforeground="#ffd166",
        cursor="hand2",
        command=toggle_favorite_mode
    )

    favorite_mode_btn.pack(side="right")

    for i in range(NUM_SLOTS):
        row = tk.Frame(slots_frame, bg=SLOT_EMPTY, pady=6, padx=10)
        row.pack(fill="x", pady=2)
        slot_rows.append(row)        

        badge = tk.Label(
            row,
            text=f"F{i+1}",
            font=FONT_BOLD,
            bg=ACCENT,
            fg="#0b1615",
            width=3,
            padx=4
        )
        badge.pack(side="left")

        coord_lbl = tk.Label(
            row,
            textvariable=state.slot_vars[i],
            font=FONT_MONO_ITALIC if state.slots[i] is None else FONT_MONO,
            bg=SLOT_EMPTY,
            fg=MUTED,
            anchor="w",
            width=38
        )
        coord_lbl.pack(side="left", padx=(10, 0))

        slot_bg = SLOT_FILLED if state.slots[i] else SLOT_EMPTY

        right_actions = tk.Frame(
            row,
            bg=slot_bg
        )
        right_actions.pack(side="right")

        row.right_actions = right_actions

        def make_clear(idx=i, frame=row, lbl=coord_lbl):

            def clear():

                create_undo_snapshot()

                if state.favorite_mode:

                    state.favorite_slots[idx] = None
                    state.favorite_names[idx] = f"Favorite {idx+1}"

                    show_notification(
                        f"🗑️ Favorite {idx+1} cleared",
                        "error"
                    )

                else:

                    state.slots[idx] = None
                    state.slot_names[idx] = f"Slot {idx+1}"
                    state.slot_times[idx] = None
                    state.slot_colors[idx] = None

                    show_notification(
                        f"🗑️ Slot {idx+1} cleared",
                        "error"
                    )

                state.slot_vars[idx].set("— empty —")

                frame.configure(bg=SLOT_EMPTY)

                lbl.configure(
                    bg=SLOT_EMPTY,
                    fg=MUTED,
                    font=FONT_MONO_ITALIC
                )

                refresh_slot_display()

                root.after(
                    0,
                    lambda idx=idx: flash_slot_safe(idx, "delete")
                )

                reset_status_after_delay()

                write_cfg()
                save_slots_to_json()

            return clear

        def make_cycle_color(idx=i, button=None):

            def cycle_color():

                if state.slots[idx] is None:
                    return

                current_color = state.slot_colors[idx]

                current_index = SLOT_TAG_COLORS.index(current_color)

                next_index = (
                    current_index + 1
                ) % len(SLOT_TAG_COLORS)

                state.slot_colors[idx] = SLOT_TAG_COLORS[next_index]

                save_slots_to_json()

                refresh_slot_display()

            return cycle_color

        color_btn = tk.Button(
            right_actions,
            text="●",
            font=("Segoe UI", 10),
            bg=SLOT_EMPTY,
            fg=MUTED,
            bd=0,
            activebackground=ACCENT,
            activeforeground="white",
            cursor="hand2"
        )

        color_btn.pack(side="left", padx=(0, 8))
        
        color_btn.configure(
            command=make_cycle_color(i, color_btn)
        )

        row.color_btn = color_btn

        favorite_btn = tk.Button(
            right_actions,
            text="☆",
            font=("Segoe UI", 10),
            bg=slot_bg,
            fg=MUTED,
            bd=0,
            activebackground=ACCENT,
            activeforeground="#ffd166",
            cursor="hand2",
            command=lambda idx=i: open_favorite_save_window(idx)
        )

        favorite_btn.pack(side="left", padx=(0, 8))

        row.favorite_btn = favorite_btn

        apply_button_hover(
            favorite_btn,
            slot_bg,
            MUTED,
            hover_bg=ACCENT,
            hover_fg="#ffd166"
        )

        rename_btn = tk.Button(
            right_actions,
            text="Rename",
            font=("Segoe UI", 8),
            bg=slot_bg,
            fg=MUTED,
            bd=0,
            activebackground=ACCENT,
            activeforeground="white",
            cursor="hand2",
            command=lambda idx=i: rename_slot(idx)
        )

        rename_btn.pack(side="left", padx=(0, 8))

        row.rename_btn = rename_btn

        apply_button_hover(
            rename_btn,
            slot_bg,
            MUTED
        )

        delete_btn = tk.Button(
            right_actions,
            text="✕",
            font=("Segoe UI", 8),
            bg=slot_bg,
            fg=MUTED,
            bd=0,
            activebackground=ACCENT,
            activeforeground="white",
            cursor="hand2",
            command=make_clear(i)
        )

        delete_btn.pack(side="left")

        row.delete_btn = delete_btn

        apply_button_hover(
            delete_btn,
            slot_bg,
            MUTED,
            hover_bg="#a83246"
        )

        def make_trace(
            idx=i,
            frame=row,
            lbl=coord_lbl,
            actions=right_actions,
            color_button=color_btn,
            fav_button=favorite_btn,
            ren_button=rename_btn,
            del_button=delete_btn
        ):
            def trace(*_):
                active_slots = (
                    state.favorite_slots
                    if state.favorite_mode
                    else state.slots
                )

                filled = active_slots[idx] is not None
                tag_color = state.slot_colors[idx]

                bg = SLOT_FILLED if filled else SLOT_EMPTY
                
                fg = "#b8d8d2" if filled else "#8fa09c"
                
                frame.configure(bg=bg)
                
                lbl.configure(
                    bg=bg,
                    fg=fg,
                    font=FONT_MONO if filled else FONT_MONO_ITALIC
                )
                
                actions.configure(
                    bg=bg
                )

                fav_button.configure(
                    bg=bg,
                    activebackground=bg
                )

                ren_button.configure(
                    bg=bg,
                    activebackground=bg
                )

                del_button.configure(
                    bg=bg,
                    activebackground=bg
                )

                color_button.configure(
                    bg=tag_color if filled and tag_color else bg,
                    fg="white" if filled and tag_color else MUTED,
                    activebackground=tag_color if filled and tag_color else bg,
                    activeforeground="white"
                )
            return trace

        state.slot_vars[i].trace_add("write", make_trace(i))
        
        def flash_slot(
            mode="save",
            idx=i,
            frame=row,
            lbl=coord_lbl
        ):

            palettes = {
                "save": [
                    "#4ecca3",
                    "#3fa98a",
                    "#2f7c6a",
                    SLOT_FILLED
                ],

                "load": [
                    "#4d7cff",
                    "#3d63cc",
                    "#2d4a99",
                    SLOT_FILLED
                ],

                "delete": [
                    "#cc4e6c",
                    "#993b52",
                    "#662737",
                    SLOT_EMPTY
                ],

                "rename": [
                    "#8b5cf6",
                    "#6d46c7",
                    "#503498",
                    SLOT_FILLED
                ]
            }

            colors = palettes.get(
                mode,
                palettes["save"]
            )

            def step_flash(step=0):

                if step < len(colors):

                    color = colors[step]

                    frame.configure(bg=color)
                    lbl.configure(bg=color)

                    root.after(
                        70,
                        lambda: step_flash(step + 1)
                    )

            step_flash()

        row.flash_slot = flash_slot

        def on_row_enter(event, idx=i, frame=row, lbl=coord_lbl):
            active_slots = (
                state.favorite_slots
                if state.favorite_mode
                else state.slots
            )

            filled = active_slots[idx] is not None
            bg = "#2b4a4d" if filled else "#223437"

            frame.configure(bg=bg)
            lbl.configure(bg=bg)

        def on_row_leave(event, idx=i, frame=row, lbl=coord_lbl):
            active_slots = (
                state.favorite_slots
                if state.favorite_mode
                else state.slots
            )

            filled = active_slots[idx] is not None
            bg = SLOT_FILLED if filled else SLOT_EMPTY

            frame.configure(bg=bg)
            lbl.configure(bg=bg)

        # row.bind("<Enter>", on_row_enter)
        # row.bind("<Leave>", on_row_leave)
        # coord_lbl.bind("<Enter>", on_row_enter)
        # coord_lbl.bind("<Leave>", on_row_leave)
        
    # ── How-to section
    help_frame = tk.Frame(root, bg=CARD_BG, pady=10, padx=14)
    help_frame.pack(fill="x", padx=20, pady=(10, 0))

    tk.Label(
        help_frame,
        text="HOW TO USE",
        font=("Segoe UI", 8, "bold"),
        bg=CARD_BG,
        fg=MUTED
    ).pack(anchor="w")

    how_to_use_vars = [
        tk.StringVar(
            value="Launch Deadlock with -condebug -consolelog in Steam launch options"
        ),

        tk.StringVar(
            value="Enable cheats in-game: sv_cheats true"
        ),

        tk.StringVar(
            value="Press F1–F8 to save your current position"
        ),

        tk.StringVar(
            value="Press Alt+F1–F8 to teleport to a saved position"
        ),

        tk.StringVar(),

        tk.StringVar(),

        tk.StringVar(),
    ]

    def refresh_how_to_use_text():

        how_to_use_vars[4].set(
            f"Press {state.preset_cycle_hotkey.upper()} to switch between presets"
        )

        how_to_use_vars[5].set(
            f"Press {state.undo_hotkey.upper()} / "
            f"{state.redo_hotkey.upper()} for Undo and Redo"
        )

        how_to_use_vars[6].set(
            f"Press {state.favorite_mode_hotkey.upper()} "
            f"to toggle Favorite mode"
        )

    refresh_how_to_use_text()

    steps = [
        ("1.", how_to_use_vars[0]),
        ("2.", how_to_use_vars[1]),
        ("3.", how_to_use_vars[2]),
        ("4.", how_to_use_vars[3]),
        ("5.", how_to_use_vars[4]),
        ("6.", how_to_use_vars[5]),
        ("7.", how_to_use_vars[6]),
    ]

    for num, text in steps:
        row = tk.Frame(help_frame, bg=CARD_BG)
        row.pack(fill="x", pady=3)

        tk.Label(
            row,
            text=num,
            font=FONT_BOLD,
            bg=CARD_BG,
            fg=ACCENT,
            width=2
        ).pack(side="left")

        tk.Label(
            row,
            textvariable=text,
            font=("Segoe UI", 9),
            bg=CARD_BG,
            fg=TEXT,
            wraplength=430,
            justify="left"
        ).pack(side="left", padx=(4, 0))
        
    good_to_know_frame = tk.Frame(
        help_frame,
        bg=CARD_BG
    )

    good_to_know_frame.pack(
        fill="x",
        pady=(8, 0)
    )

    tk.Label(
        good_to_know_frame,
        text="Good to Know:",
        font=("Segoe UI", 8, "bold", "underline"),
        bg=CARD_BG,
        fg=ACCENT
    ).pack(
        anchor="w"
    )

    line1 = tk.Frame(
        good_to_know_frame,
        bg=CARD_BG
    )

    line1.pack(
        anchor="w",
        pady=(1, 0)
    )

    tk.Label(
        line1,
        text="If you jump into",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line1,
        text="NYC",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line1,
        text="immediately after launching",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line1,
        text="Deadlock",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line1,
        text=",",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line1,
        text="SPLIT",
        font=("Segoe UI", 8, "bold"),
        bg=CARD_BG,
        fg=ACCENT,
        padx=-2
    ).pack(side="left")

    tk.Label(
        line1,
        text="may need a few seconds before working reliably.",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    line2 = tk.Frame(
        good_to_know_frame,
        bg=CARD_BG
    )

    line2.pack(
        anchor="w",
        pady=(1, 0)
    )

    tk.Label(
        line2,
        text="During this short warmup period, pressing ",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line2,
        text="F1-F8",
        font=("Segoe UI", 8, "bold"),
        bg=ACCENT,
        fg="#0b1615",
        padx=1
    ).pack(side="left")

    tk.Label(
        line2,
        text=" may occasionally show:",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        good_to_know_frame,
        text='"No getpos response — check -condebug -consolelog"',
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=WARN,
        wraplength=610,
        justify="left"
    ).pack(
        anchor="w"
    )

    line3 = tk.Frame(
        good_to_know_frame,
        bg=CARD_BG
    )

    line3.pack(
        anchor="w",
        pady=(4, 0)
    )

    tk.Label(
        line3,
        text="If your hotkeys still don't respond after a while, try restarting ",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        line3,
        text="SPLIT",
        font=("Segoe UI", 8, "bold"),
        bg=CARD_BG,
        fg=ACCENT,
        padx=-2
    ).pack(side="left")

    tk.Label(
        line3,
        text=".",
        font=("Segoe UI", 8),
        bg=CARD_BG,
        fg=MUTED
    ).pack(side="left")

    tk.Label(
        root,
        text=f"{APP_NAME} {APP_VERSION}",
        font=("Segoe UI", 7),
        bg=DARK_BG,
        fg=MUTED
    ).pack(pady=(6, 4))

    def on_close():

        state.running = False

        try:
            keyboard.unhook_all_hotkeys()
            keyboard.unhook_all()

        except Exception as e:
            debug_error("on_close keyboard cleanup", e)

        registered_hotkeys.clear()

        debug_log("Application closing")

        save_slots_to_json()
        save_app_config()
        debug_log("Application closing")

        root.destroy()

    def refresh_saved_times():

        refresh_slot_display()

        root.after(
            30000,
            refresh_saved_times
        )

    refresh_saved_times()

    root.protocol("WM_DELETE_WINDOW", on_close)

    return root
# Startup sound
def play_boot_sound():

    if not state.boot_sound_enabled:
        return

    try:

        winsound.PlaySound(
            resource_path("boot.wav"),
            winsound.SND_FILENAME | winsound.SND_ASYNC
        )

    except Exception:
        pass

# Admin check
def is_running_as_admin():

    try:
        return ctypes.windll.shell32.IsUserAnAdmin()

    except Exception:
        return False

#Double Instance
def prevent_multiple_instances():

    mutex = ctypes.windll.kernel32.CreateMutexW(
        None,
        False,
        SINGLE_INSTANCE_MUTEX_NAME
    )

    last_error = ctypes.windll.kernel32.GetLastError()

    if last_error == 183:

        ask_confirm_window(
            "SPLIT Already Running",
            "Close the existing window first.",
            confirm_text="OK",
            danger=True,
            cancel_text="Close",
            center_on_screen=True
        )

        sys.exit()

    return mutex

# Main
def main():

    # Create Tk root FIRST
    global root
    root = tk.Tk()

    apply_window_icon(root)

    # Hide window during admin check
    root.withdraw()   

    is_frozen = getattr(
        sys,
        "frozen",
        False
    )

    if is_frozen and not is_running_as_admin():

        ctypes.windll.shell32.ShellExecuteW(
            None,
            "runas",
            sys.executable,
            " ".join(sys.argv),
            None,
            1
        )

        sys.exit()
        
    app_mutex = prevent_multiple_instances()

    debug_log("Application started")    

    # Show app window
    root.attributes("-alpha", 0.0)

    root.deiconify()
    
    global state
    state = AppState()
    
    load_app_config()
    load_slots_from_json()
    
    # Write initial empty cfg + patch autoexec
    try:
        write_cfg()
        ensure_autoexec()
    except Exception as e:
        print(f"[WARN] Could not write cfg: {e}")

    # Start log watcher thread
    state.log_thread = threading.Thread(target=tail_log, daemon=True)
    state.log_thread.start()

    # Build GUI
    build_gui(root)
    play_boot_sound()
    
    def fade_in(step=0):

        alpha = step / 20

        if alpha > 1:
            alpha = 1

        root.attributes(
            "-alpha",
            alpha
        )

        if alpha < 1:

            root.after(
                16,
                lambda: fade_in(step + 1)
            )

    fade_in()    

    # Register hotkeys
    try:
        setup_hotkeys()

    except Exception as e:
        print(f"[WARN] Hotkeys: {e}")

    root.mainloop()


if __name__ == "__main__":
    main()
