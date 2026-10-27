use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Manager,
};

static TRAY_LANGUAGE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const TRAY_LABELS: [[&str; 5]; 8] = [["Deadlock: Connected","Deadlock: Not running","Launch Deadlock","Open SPLIT","Quit SPLIT"],["Deadlock : connecté","Deadlock : non lancé","Lancer Deadlock","Ouvrir SPLIT","Quitter SPLIT"],["Deadlock: conectado","Deadlock: no iniciado","Iniciar Deadlock","Abrir SPLIT","Salir de SPLIT"],["Deadlock: verbunden","Deadlock: nicht gestartet","Deadlock starten","SPLIT öffnen","SPLIT beenden"],["Deadlock: conectado","Deadlock: não iniciado","Iniciar Deadlock","Abrir SPLIT","Sair do SPLIT"],["Deadlock: подключён","Deadlock: не запущен","Запустить Deadlock","Открыть SPLIT","Выйти из SPLIT"],["Deadlock：接続済み","Deadlock：未起動","Deadlockを起動","SPLITを開く","SPLITを終了"],["Deadlock：已连接","Deadlock：未运行","启动 Deadlock","打开 SPLIT","退出 SPLIT"]];
fn tray_label(index: usize) -> &'static str {
    TRAY_LABELS[TRAY_LANGUAGE.load(std::sync::atomic::Ordering::Relaxed)][index]
}

pub fn set_language(app: &tauri::AppHandle, language: &str) {
    let index = match language {
        "fr" => 1, "es" => 2, "de" => 3, "pt" => 4,
        "ru" => 5, "ja" => 6, "zh" => 7, _ => 0,
    };
    TRAY_LANGUAGE.store(index, std::sync::atomic::Ordering::Relaxed);
    if let Some(tray) = app.tray_by_id("split-tray") {
        if let Ok(menu) = make_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn make_menu<R: tauri::Runtime>(app: &impl tauri::Manager<R>) -> Result<tauri::menu::Menu<R>, String> {
    let running = crate::deadlock::is_deadlock_running();
    let status = MenuItemBuilder::with_id(DEADLOCK_STATUS_ID, tray_label(if running { 0 } else { 1 }))
        .enabled(false).build(app).map_err(|e| e.to_string())?;
    let launch = MenuItemBuilder::with_id(LAUNCH_DEADLOCK_ID, tray_label(2))
        .enabled(!running).build(app).map_err(|e| e.to_string())?;
    MenuBuilder::new(app).item(&status).separator().text(OPEN_ID, tray_label(3))
        .item(&launch).separator().text(QUIT_ID, tray_label(4))
        .build().map_err(|e| e.to_string())
}

const OPEN_ID: &str = "open-split";

const LAUNCH_DEADLOCK_ID: &str = "launch-deadlock";

const QUIT_ID: &str = "quit-split";

const DEADLOCK_STATUS_ID: &str = "deadlock-status";

pub fn setup(app: &App) -> Result<(), String> {
    let deadlock_running = crate::deadlock::is_deadlock_running();

    let status_item = MenuItemBuilder::with_id(
        DEADLOCK_STATUS_ID,
        if deadlock_running {
            tray_label(0)
        } else {
            tray_label(1)
        },
    )
    .enabled(false)
    .build(app)
    .map_err(|error| format!("Could not create Deadlock tray status: {error}"))?;

    let launch_item = MenuItemBuilder::with_id(LAUNCH_DEADLOCK_ID, tray_label(2))
        .enabled(!deadlock_running)
        .build(app)
        .map_err(|error| format!("Could not create Deadlock tray action: {error}"))?;

    let menu = MenuBuilder::new(app)
        .item(&status_item)
        .separator()
        .text(OPEN_ID, tray_label(3))
        .item(&launch_item)
        .separator()
        .text(QUIT_ID, tray_label(4))
        .build()
        .map_err(|error| format!("Could not build tray menu: {error}"))?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "SPLIT tray icon is unavailable".to_string())?;

    let status_item_for_tray = status_item.clone();

    let launch_item_for_tray = launch_item.clone();

    TrayIconBuilder::with_id("split-tray")
        .icon(icon)
        .tooltip("SPLIT")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_ID => {
                if let Err(error) = crate::app_window::open_main_window(app.clone()) {
                    eprintln!("[SPLIT] Could not open window from tray: {error}");
                }
            }
            LAUNCH_DEADLOCK_ID => {
                if let Err(error) = crate::deadlock::launch_deadlock() {
                    eprintln!("[SPLIT] Could not launch Deadlock from tray: {error}");
                }
            }
            QUIT_ID => crate::app_window::request_true_quit(app),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                ..
            } => {
                let running = crate::deadlock::is_deadlock_running();

                let _ = status_item_for_tray.set_text(if running {
                    tray_label(0)
                } else {
                    tray_label(1)
                });

                let _ = launch_item_for_tray.set_enabled(!running);
            }

            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                if let Err(error) = crate::app_window::open_main_window(tray.app_handle().clone()) {
                    eprintln!("[SPLIT] Could not open window from tray click: {error}");
                }
            }

            _ => {}
        })
        .build(app)
        .map_err(|error| format!("Could not create tray icon: {error}"))?;

    Ok(())
}
