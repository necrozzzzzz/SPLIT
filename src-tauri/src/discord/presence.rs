use std::{
    sync::{mpsc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{
    activity::{Activity, Assets, Party, Timestamps},
    DiscordIpc, DiscordIpcClient,
};

use super::state::{presence_changed, PresenceModel, PresenceSnapshot, ResolvedPhase};
use crate::deadlock::{
    console_phase::{ConsolePhase, ConsolePhaseState},
    deadlock_state::DeadlockState,
};

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
    let mut assets = Assets::new();
    let mut has_assets = false;

    if snapshot.show_large_image {
        assets = assets
            .large_image(snapshot.large_image)
            .large_text(snapshot.large_text);
        has_assets = true;
    }

    if let Some(small_image) = snapshot.small_image {
        assets = assets.small_image(small_image);
        has_assets = true;
    }
    if let Some(small_text) = snapshot.small_text {
        assets = assets.small_text(small_text);
    }

    let mut activity = Activity::new().details(snapshot.details.clone());

    if snapshot.show_state {
        activity = activity.state(snapshot.state.clone());
    }
    if has_assets {
        activity = activity.assets(assets);
    }

    if snapshot.show_party {
        activity = activity.party(Party::new().size([
            snapshot.party_size.min(i32::MAX as u32) as i32,
            snapshot.party_max.min(i32::MAX as u32) as i32,
        ]));
    }

    if let Some(started_at) = snapshot.started_at {
        activity = activity.timestamps(Timestamps::new().start(started_at));
    }

    activity
}

struct WorkerObservation<'a> {
    deadlock_running: bool,
    memory: Option<DeadlockState>,
    console: &'a ConsolePhaseState,
    district_raw: Option<i8>,
    unix_now: i64,
    monotonic_now: Instant,
    config: &'a super::DiscordPresenceConfig,
}

fn observe_worker_tick(
    model: &mut PresenceModel,
    observation: WorkerObservation<'_>,
) -> Option<PresenceSnapshot> {
    model.observe_with_console_and_district_at_config(
        observation.deadlock_running,
        observation.memory,
        observation.console,
        observation.district_raw,
        observation.unix_now,
        observation.monotonic_now,
        observation.config,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PresenceDebugState {
    console_phase: ConsolePhase,
    resolved_phase: Option<ResolvedPhase>,
    previous_valid_phase: Option<ResolvedPhase>,
    transition_active: bool,
    map: Option<String>,
    hero: Option<String>,
    spectating_match_id: Option<u64>,
    details: Option<String>,
    state: Option<String>,
    large_image: Option<&'static str>,
}

fn debug_state(
    console: &ConsolePhaseState,
    previous_valid_phase: Option<ResolvedPhase>,
    model: &PresenceModel,
    desired: Option<&PresenceSnapshot>,
) -> PresenceDebugState {
    PresenceDebugState {
        console_phase: console.phase,
        resolved_phase: desired.map(|snapshot| snapshot.resolved_phase),
        previous_valid_phase,
        transition_active: model.transition_active(),
        map: console.current_map.clone(),
        hero: console.current_hero.clone(),
        spectating_match_id: console.spectating_match_id,
        details: desired.map(|snapshot| snapshot.details.clone()),
        state: desired.map(|snapshot| snapshot.state.clone()),
        large_image: desired.map(|snapshot| snapshot.large_image),
    }
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

fn log_presence_debug(state: &PresenceDebugState) {
    println!(
        "[SPLIT][PresenceDebug] console_phase={:?} resolved_phase={:?} previous_valid_phase={:?} transition_active={} map={:?} hero={:?} spectating_match_id={:?} details={:?} state={:?} large_image={:?}",
        state.console_phase,
        state.resolved_phase,
        state.previous_valid_phase,
        state.transition_active,
        state.map,
        state.hero,
        state.spectating_match_id,
        state.details,
        state.state,
        state.large_image,
    );
}

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
    let mut last_debug_state: Option<PresenceDebugState> = None;
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
        let observed_at = model.session_started_at().unwrap_or_else(unix_now);
        let district = crate::deadlock::district::current_raw();
        let previous_valid_phase = model.last_known_phase();
        let config = super::config();
        let desired = observe_worker_tick(
            &mut model,
            WorkerObservation {
                deadlock_running,
                memory,
                console: &console,
                district_raw: district,
                unix_now: observed_at,
                monotonic_now: Instant::now(),
                config: &config,
            },
        );

        let current_debug_state =
            debug_state(&console, previous_valid_phase, &model, desired.as_ref());
        if last_debug_state.as_ref() != Some(&current_debug_state) {
            log_presence_debug(&current_debug_state);
            last_debug_state = Some(current_debug_state);
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadlock::deadlock_state::{DeadlockActivity, GameMode, MatchMode};
    use crate::discord::{DiscordPresenceConfig, PartyDisplay};

    fn idle_memory() -> DeadlockState {
        DeadlockState {
            activity: DeadlockActivity::Idle,
            match_mode: MatchMode::Invalid,
            game_mode: GameMode::Invalid,
            party_size: 1,
            party_max: 6,
        }
    }

    fn console(lines: &[&str]) -> ConsolePhaseState {
        crate::deadlock::console_phase::state_from_test_lines(lines.iter().copied())
    }

    fn tick(
        model: &mut PresenceModel,
        console: &ConsolePhaseState,
        district_raw: Option<i8>,
        unix_now: i64,
        monotonic_now: Instant,
    ) -> PresenceSnapshot {
        tick_with_config(
            model,
            console,
            district_raw,
            unix_now,
            monotonic_now,
            &DiscordPresenceConfig::default(),
        )
    }

    fn tick_with_config(
        model: &mut PresenceModel,
        console: &ConsolePhaseState,
        district_raw: Option<i8>,
        unix_now: i64,
        monotonic_now: Instant,
        config: &DiscordPresenceConfig,
    ) -> PresenceSnapshot {
        observe_worker_tick(
            model,
            WorkerObservation {
                deadlock_running: true,
                memory: Some(idle_memory()),
                console,
                district_raw,
                unix_now,
                monotonic_now,
                config,
            },
        )
        .unwrap()
    }

    #[test]
    fn worker_runtime_path_publishes_explore_hero_district_and_asset() {
        let explore = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_midtown\"",
            "[Client] Created physics for dl_midtown",
            "[Server] Loaded hero 2862/hero_ratking",
        ]);
        let start = Instant::now();
        let mut model = PresenceModel::default();

        let docks = tick(&mut model, &explore, Some(3), 100, start);
        assert_eq!(docks.details, "Exploring NYC with Rat King");
        assert_eq!(docks.state, "› York : Docks");
        assert_eq!(docks.large_image, "york_docks");

        let activity = serde_json::to_value(activity_for(&docks)).unwrap();
        assert_eq!(activity["details"], "Exploring NYC with Rat King");
        assert_eq!(activity["state"], "› York : Docks");
        assert_eq!(activity["assets"]["large_image"], "york_docks");
        assert!(activity.get("party").is_none());

        let uptown = tick(
            &mut model,
            &explore,
            Some(15),
            101,
            start + Duration::from_secs(1),
        );
        assert_eq!(uptown.details, "Exploring NYC with Rat King");
        assert_eq!(uptown.state, "› Broadway : Uptown");
        assert_eq!(uptown.large_image, "broadway_uptown");

        let explore_without_hero = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_midtown\"",
            "[Client] Created physics for dl_midtown",
        ]);
        let no_hero = tick(
            &mut model,
            &explore_without_hero,
            Some(3),
            102,
            start + Duration::from_secs(2),
        );
        assert_eq!(no_hero.details, "Exploring NYC");
        assert_eq!(no_hero.state, "› York : Docks");
        assert_eq!(no_hero.large_image, "york_docks");

        let activity = serde_json::to_value(activity_for(&no_hero)).unwrap();
        assert_eq!(activity["details"], "Exploring NYC");
        assert!(activity.get("party").is_none());
    }

    #[test]
    fn worker_model_persists_loading_across_real_runtime_transition_signals() {
        let explore = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_midtown\"",
            "[Client] Created physics for dl_midtown",
            "[Server] Loaded hero 2862/hero_ratking",
        ]);
        let transient_disconnect =
            console(&["[Client] Disconnecting from server: NETWORK_DISCONNECT_LOOPDEACTIVATE"]);
        let hideout = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_hideout\"",
            "[Client] Created physics for dl_hideout",
            "[Server] Loaded hero 3097/hero_ratking",
        ]);
        let start = Instant::now();
        let mut model = PresenceModel::default();

        let initial = tick(&mut model, &explore, Some(3), 100, start);
        assert_eq!(initial.resolved_phase, ResolvedPhase::ExploreNyc);

        let loading = tick(
            &mut model,
            &transient_disconnect,
            None,
            101,
            start + Duration::from_secs(1),
        );
        assert_eq!(loading.resolved_phase, ResolvedPhase::TransitionLoading);
        assert_eq!(
            (loading.details.as_str(), loading.state.as_str()),
            ("Deadlock", "Loading...")
        );
        assert_eq!(loading.large_image, "deadlock_logo");
        assert!(model.transition_active());
        assert_eq!(model.last_known_phase(), Some(ResolvedPhase::ExploreNyc));

        let still_loading = tick(
            &mut model,
            &transient_disconnect,
            None,
            104,
            start + Duration::from_secs(4),
        );
        assert_eq!(
            still_loading.resolved_phase,
            ResolvedPhase::TransitionLoading
        );

        let resolved = tick(
            &mut model,
            &hideout,
            None,
            105,
            start + Duration::from_millis(4_500),
        );
        assert_eq!(resolved.resolved_phase, ResolvedPhase::Hideout);
        assert_eq!(resolved.details, "Wishing the Hideout was on Long Island");
        assert_eq!(resolved.large_image, "rat_king");
        assert!(!model.transition_active());

        let activity = serde_json::to_value(activity_for(&resolved)).unwrap();
        assert_eq!(activity["party"]["size"], serde_json::json!([1, 6]));
    }

    #[test]
    fn worker_runtime_serializes_hltv_match_id_without_hero_district_or_party() {
        let spectating = console(&[
            "[GCClient] Send msg 9109 (k_EMsgClientToGCSpectateLobby), 24 bytes",
            "[HLTV Broadcast] Stream State Stop -> Sync",
            "[HostStateManager] Playing Broadcast (http://dist1-ord1.steamcontent.com/tv/110501755_856385445)",
            "[Client] Created physics for dl_midtown",
            "ChangeGameState: GameInProgress (7)",
            "VMDL Camera Pose Success! models/heroes/bookworm/hero.vmdl",
        ]);
        let mut config = DiscordPresenceConfig::default();
        config.spectating.party_display = PartyDisplay::Discord;

        let result = tick_with_config(
            &mut PresenceModel::default(),
            &spectating,
            Some(8),
            100,
            Instant::now(),
            &config,
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::Spectating);
        assert_eq!(result.details, "Spectating a game");
        assert_eq!(result.state, "Match 110501755");
        assert!(result.show_state);
        assert_eq!(result.current_hero, None);
        assert_eq!(result.large_image, "deadlock_logo");
        assert!(!result.show_party);
        assert_eq!(spectating.spectating_match_id, Some(110501755));

        let activity = serde_json::to_value(activity_for(&result)).unwrap();
        assert_eq!(activity["details"], "Spectating a game");
        assert_eq!(activity["state"], "Match 110501755");
        assert_eq!(activity["assets"]["large_image"], "deadlock_logo");
        assert!(activity.get("party").is_none());

        config.spectating.show_match_id = false;
        config.spectating.party_display = PartyDisplay::Compact;
        let hidden_id = tick_with_config(
            &mut PresenceModel::default(),
            &spectating,
            Some(8),
            100,
            Instant::now(),
            &config,
        );
        assert_eq!(hidden_id.details, "Spectating a game");
        assert!(!hidden_id.show_state);
        assert!(!hidden_id.show_party);
        let activity = serde_json::to_value(activity_for(&hidden_id)).unwrap();
        assert!(activity.get("state").is_none());
        assert!(activity.get("party").is_none());
    }

    #[test]
    fn worker_runtime_match_loading_hideout_sequence_keeps_hero_and_exits_loading() {
        let mut console = console(&[
            "[GCClient] Recv msg 9100 (k_EMsgGCToClientSDRTicket), 384 bytes",
            "[Client] CL: Connected to '203.0.113.8:27015'",
            "[Server] Loaded hero 2862/hero_ratking",
            "ChangeGameState: GameInProgress (7)",
        ]);
        let start = Instant::now();
        let mut model = PresenceModel::default();

        let in_match = tick(&mut model, &console, None, 100, start);
        assert_eq!(in_match.resolved_phase, ResolvedPhase::InMatch);
        assert_eq!(
            (in_match.details.as_str(), in_match.state.as_str()),
            ("In Match", "Playing as Rat King · 1/6")
        );
        assert_eq!(in_match.current_hero.as_deref(), Some("ratking"));
        assert_eq!(in_match.large_image, "rat_king");
        assert!(!in_match.show_party);
        assert!(serde_json::to_value(activity_for(&in_match))
            .unwrap()
            .get("party")
            .is_none());

        let mut discord_config = DiscordPresenceConfig::default();
        discord_config.r#match.party_display = PartyDisplay::Discord;
        let discord_party = tick_with_config(
            &mut PresenceModel::default(),
            &console,
            None,
            100,
            start,
            &discord_config,
        );
        assert_eq!(discord_party.state, "Playing as Rat King");
        assert!(discord_party.show_party);
        assert_eq!(
            serde_json::to_value(activity_for(&discord_party)).unwrap()["party"]["size"],
            serde_json::json!([1, 6])
        );

        crate::deadlock::console_phase::parse_line(&mut console, "ChangeGameState: PostGame (6)");
        let post_match = tick(
            &mut model,
            &console,
            None,
            101,
            start + Duration::from_secs(1),
        );
        assert_eq!(post_match.resolved_phase, ResolvedPhase::PostMatch);

        crate::deadlock::console_phase::parse_line(
            &mut console,
            "[Client] Disconnecting from server: NETWORK_DISCONNECT_LOOPDEACTIVATE",
        );
        let loading = tick(
            &mut model,
            &console,
            None,
            102,
            start + Duration::from_secs(2),
        );
        assert_eq!(loading.resolved_phase, ResolvedPhase::TransitionLoading);
        assert_eq!(
            (loading.details.as_str(), loading.state.as_str()),
            ("Deadlock", "Loading...")
        );
        assert_eq!(loading.large_image, "deadlock_logo");

        crate::deadlock::console_phase::parse_line(
            &mut console,
            "[Client] Created physics for dl_hideout",
        );
        crate::deadlock::console_phase::parse_line(
            &mut console,
            "[Server] Loaded hero 3097/hero_ratking",
        );
        let hideout = tick(
            &mut model,
            &console,
            None,
            103,
            start + Duration::from_millis(2_100),
        );

        assert_eq!(hideout.resolved_phase, ResolvedPhase::Hideout);
        assert_eq!(hideout.details, "Wishing the Hideout was on Long Island");
        assert_eq!(hideout.state, "Hideout");
        assert_eq!(hideout.large_image, "rat_king");
        assert!(!model.transition_active());

        assert_eq!(hideout.started_at, Some(100));

        // Un LOOPDEACTIVATE retardataire peut encore arriver après que
        // dl_hideout a déjà été confirmé. Il ne doit pas refaire passer
        // le Rich Presence en Loading ni effacer le héros du Hideout.
        crate::deadlock::console_phase::parse_line(
            &mut console,
            "[Client] Disconnecting from server: NETWORK_DISCONNECT_LOOPDEACTIVATE",
        );

        let after_late_disconnect = tick(
            &mut model,
            &console,
            None,
            104,
            start + Duration::from_millis(2_200),
        );

        assert_eq!(after_late_disconnect.resolved_phase, ResolvedPhase::Hideout);
        assert_eq!(
            after_late_disconnect.details,
            "Wishing the Hideout was on Long Island"
        );
        assert_eq!(after_late_disconnect.state, "Hideout");
        assert_eq!(
            after_late_disconnect.current_hero.as_deref(),
            Some("ratking")
        );
        assert_eq!(after_late_disconnect.large_image, "rat_king");
        assert_eq!(after_late_disconnect.started_at, Some(100));
        assert!(!model.transition_active());

        let activity = serde_json::to_value(activity_for(&after_late_disconnect)).unwrap();

        assert_eq!(
            activity["details"],
            "Wishing the Hideout was on Long Island"
        );
        assert_eq!(activity["state"], "Hideout");
        assert_eq!(activity["assets"]["large_image"], "rat_king");
    }

    #[test]
    fn worker_runtime_loading_expires_and_stable_menu_does_not_start_it() {
        let explore = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_midtown\"",
            "[Server] Loaded hero 2862/hero_ratking",
        ]);
        let transient_disconnect =
            console(&["[Client] Disconnecting from server: NETWORK_DISCONNECT_LOOPDEACTIVATE"]);
        let stable_menu = console(&["LoopMode: menu"]);
        let start = Instant::now();
        let mut model = PresenceModel::default();

        tick(&mut model, &explore, Some(3), 100, start);
        tick(
            &mut model,
            &transient_disconnect,
            None,
            101,
            start + Duration::from_secs(1),
        );
        let expired = tick(
            &mut model,
            &transient_disconnect,
            None,
            106,
            start + Duration::from_secs(6),
        );
        assert_eq!(expired.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(
            (expired.details.as_str(), expired.state.as_str()),
            ("Deadlock", "In Menu")
        );

        let mut fresh_model = PresenceModel::default();
        let menu = tick(&mut fresh_model, &stable_menu, None, 200, start);
        assert_eq!(menu.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(menu.state, "In Menu");
        assert!(!fresh_model.transition_active());
    }

    #[test]
    fn explore_nyc_configuration_controls_text_artwork_and_party_serialization() {
        let explore = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_midtown\"",
            "[Client] Created physics for dl_midtown",
            "[Server] Loaded hero 2862/hero_ratking",
        ]);
        let start = Instant::now();

        let render = |config: &DiscordPresenceConfig, district_raw| {
            let mut model = PresenceModel::default();
            tick_with_config(&mut model, &explore, district_raw, 100, start, config)
        };

        let defaults = DiscordPresenceConfig::default();
        let current = render(&defaults, Some(6));
        assert_eq!(current.details, "Exploring NYC with Rat King");
        assert_eq!(current.state, "› Haunted Lot");
        assert_eq!(current.large_image, "haunted_lot");
        assert_eq!(current.started_at, Some(100));
        assert!(serde_json::to_value(activity_for(&current))
            .unwrap()
            .get("party")
            .is_none());

        let mut no_prefix = defaults.clone();
        no_prefix.explore_nyc.district_prefix.clear();
        assert_eq!(render(&no_prefix, Some(6)).state, "Haunted Lot");

        let mut bullet_prefix = defaults.clone();
        bullet_prefix.explore_nyc.district_prefix = "• ".to_string();
        assert_eq!(render(&bullet_prefix, Some(6)).state, "• Haunted Lot");

        let mut no_hero = defaults.clone();
        no_hero.explore_nyc.show_hero_in_details = false;
        assert_eq!(render(&no_hero, Some(6)).details, "Exploring NYC");

        let mut no_district = defaults.clone();
        no_district.explore_nyc.show_district = false;
        assert_eq!(render(&no_district, Some(6)).state, "Explore NYC");

        let mut no_district_image = defaults;
        no_district_image.explore_nyc.show_district_image = false;
        assert_eq!(
            render(&no_district_image, Some(6)).large_image,
            "explore_nyc_default"
        );
    }

    #[test]
    fn hideout_configuration_can_disable_official_phrase_and_party() {
        let hideout = console(&[
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_hideout\"",
            "[Client] Created physics for dl_hideout",
            "[Server] Loaded hero 3097/hero_ratking",
        ]);
        let mut config = DiscordPresenceConfig::default();
        config.hideout.use_official_hero_phrase = false;
        config.hideout.party_display = PartyDisplay::Hidden;
        let mut model = PresenceModel::default();

        let snapshot = tick_with_config(&mut model, &hideout, None, 100, Instant::now(), &config);
        assert_eq!(snapshot.details, "Deadlock");
        assert_eq!(snapshot.state, "Hideout");
        assert_eq!(snapshot.large_image, "rat_king");
        assert!(serde_json::to_value(activity_for(&snapshot))
            .unwrap()
            .get("party")
            .is_none());
    }
}
