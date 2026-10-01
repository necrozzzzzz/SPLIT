use std::{
    sync::{mpsc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{
    activity::{Activity, Party, Timestamps},
    DiscordIpc, DiscordIpcClient,
};

use super::state::{presence_changed, PresenceModel, PresenceSnapshot};


const DISCORD_APPLICATION_ID: &str = "1555044372599541930";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(10);

enum PresenceCommand {
    SetEnabled(bool),
    Stop,
}

struct PresenceRuntime {
    commands: mpsc::Sender<PresenceCommand>,
    thread: JoinHandle<()>,
}

static PRESENCE_RUNTIME: Mutex<Option<PresenceRuntime>> = Mutex::new(None);

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or_default()
}

fn activity_for(snapshot: &PresenceSnapshot) -> Activity<'static> {
    let mut activity = Activity::new()
        .details(snapshot.details)
        .state(snapshot.state)
        .party(Party::new().size([
            snapshot.party_size.min(i32::MAX as u32) as i32,
            snapshot.party_max.min(i32::MAX as u32) as i32,
        ]));

    if let Some(started_at) = snapshot.started_at {
        activity = activity.timestamps(Timestamps::new().start(started_at));
    }

    activity
}

#[cfg(debug_assertions)]
fn log_connected() {
    println!("[SPLIT][Debug] Discord Presence connected");
}

#[cfg(not(debug_assertions))]
fn log_connected() {}

#[cfg(debug_assertions)]
fn log_disconnected(reason: &str) {
    eprintln!("[SPLIT][Debug] Discord Presence disconnected: {reason}");
}

#[cfg(not(debug_assertions))]
fn log_disconnected(_reason: &str) {}

#[cfg(debug_assertions)]
fn log_deadlock_state_error(reason: &str) {
    eprintln!("[SPLIT][Debug] Discord Presence could not read Deadlock state: {reason}");
}

#[cfg(not(debug_assertions))]
fn log_deadlock_state_error(_reason: &str) {}


fn close_client(client: &mut Option<DiscordIpcClient>, clear: bool) {
    let Some(mut current) = client.take() else {
        return;
    };

    if clear {
        let _ = current.clear_activity();
    }
    let _ = current.close();
}

fn run_worker(commands: mpsc::Receiver<PresenceCommand>, initial_enabled: bool) {
    let mut enabled = initial_enabled;
    let mut model = PresenceModel::default();
    let mut client: Option<DiscordIpcClient> = None;
    let mut published: Option<PresenceSnapshot> = None;
    let mut next_connection_attempt = Instant::now();
    let mut last_read_error: Option<String> = None;
    let mut discord_unavailable_logged = false;

    loop {
        let deadlock_running = enabled && crate::deadlock::is_deadlock_running();
        let memory = if deadlock_running {
            match crate::deadlock::deadlock_state::read_deadlock_state() {
                Ok(state) => {
                    last_read_error = None;
                    state
                }
                Err(error) => {
                    if last_read_error.as_deref() != Some(&error) {
                        log_deadlock_state_error(&error);
                        last_read_error = Some(error);
                    }
                    None
                }
            }
        } else {
            None
        };
        let console = crate::deadlock::console_phase::snapshot();
        let desired = model.observe_with_console(deadlock_running, memory, &console, unix_now());

        

        if !enabled || desired.is_none() {
            if client.is_some() {
                close_client(&mut client, published.is_some());
            }
            published = None;
            discord_unavailable_logged = false;
        } else {
            if client.is_none() && Instant::now() >= next_connection_attempt {
                let mut candidate = DiscordIpcClient::new(DISCORD_APPLICATION_ID);
                match candidate.connect() {
                    Ok(()) => {
                        log_connected();
                        client = Some(candidate);
                        published = None;
                        discord_unavailable_logged = false;
                    }
                    Err(error) => {
                        if !discord_unavailable_logged {
                            log_disconnected(&error.to_string());
                            discord_unavailable_logged = true;
                        }
                        next_connection_attempt = Instant::now() + RECONNECT_INTERVAL;
                    }
                }
            }

            if presence_changed(published.as_ref(), desired.as_ref()) {
                let publish_result = match (client.as_mut(), desired.as_ref()) {
                    (Some(connected_client), Some(snapshot)) => Some((
                        connected_client.set_activity(activity_for(snapshot)),
                        snapshot,
                    )),
                    _ => None,
                };

                if let Some((result, snapshot)) = publish_result {
                    match result {
                        Ok(()) => {
                            published = Some(snapshot.clone());
                        }
                        Err(error) => {
                            log_disconnected(&error.to_string());
                            close_client(&mut client, false);
                            published = None;
                            next_connection_attempt = Instant::now() + RECONNECT_INTERVAL;
                        }
                    }
                }
            }
        }

        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(PresenceCommand::SetEnabled(next)) => {
                enabled = next;
                next_connection_attempt = Instant::now();
            }
            Ok(PresenceCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    close_client(&mut client, true);
}

pub(crate) fn start(enabled: bool) -> Result<(), String> {
    let mut runtime = PRESENCE_RUNTIME
        .lock()
        .map_err(|_| "Discord Presence runtime lock poisoned".to_string())?;

    if runtime.is_some() {
        return Ok(());
    }

    let (command_tx, command_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("split-discord-presence".to_string())
        .spawn(move || run_worker(command_rx, enabled))
        .map_err(|error| format!("Could not start Discord Presence worker: {error}"))?;

    *runtime = Some(PresenceRuntime {
        commands: command_tx,
        thread,
    });
    Ok(())
}

pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    let runtime = PRESENCE_RUNTIME
        .lock()
        .map_err(|_| "Discord Presence runtime lock poisoned".to_string())?;

    let Some(runtime) = runtime.as_ref() else {
        return Ok(());
    };

    runtime
        .commands
        .send(PresenceCommand::SetEnabled(enabled))
        .map_err(|_| "Discord Presence worker is unavailable".to_string())
}

pub(crate) fn stop() -> Result<(), String> {
    let runtime = PRESENCE_RUNTIME
        .lock()
        .map_err(|_| "Discord Presence runtime lock poisoned".to_string())?
        .take();

    if let Some(runtime) = runtime {
        let _ = runtime.commands.send(PresenceCommand::Stop);
        runtime
            .thread
            .join()
            .map_err(|_| "Discord Presence worker panicked while stopping".to_string())?;
    }

    Ok(())
}
