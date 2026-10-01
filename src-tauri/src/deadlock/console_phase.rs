use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{net::SocketAddr, str::FromStr};

use regex::{Regex, RegexBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsolePhase {
    MainMenu,
    Hideout,
    Loading,
    InMatch,
    PostMatch,
    Spectating,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerKind {
    Unknown,
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsolePhaseState {
    pub phase: ConsolePhase,
    pub current_map: Option<String>,
    pub server_kind: ServerKind,
    pub matchmaking_active: bool,
    pub matchmaking_observed: bool,
    pub match_found: bool,
    pub evidence: Option<String>,
    pub last_update_ms: u64,
}

impl ConsolePhaseState {
    fn main_menu(evidence: &str) -> Self {
        Self {
            phase: ConsolePhase::MainMenu,
            current_map: None,
            server_kind: ServerKind::Unknown,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            evidence: Some(evidence.to_string()),
            last_update_ms: now_ms(),
        }
    }

    fn set_phase(&mut self, phase: ConsolePhase, evidence: impl Into<String>) {
        self.phase = phase;
        self.evidence = Some(evidence.into());
        self.last_update_ms = now_ms();
    }

    fn set_matchmaking(&mut self, active: bool, evidence: &str) {
        if self.matchmaking_observed && self.matchmaking_active == active {
            return;
        }

        self.matchmaking_active = active;
        self.matchmaking_observed = true;
        self.evidence = Some(evidence.to_string());
        self.last_update_ms = now_ms();
    }

    fn clear_transient_match_state(&mut self) {
        self.matchmaking_active = false;
        self.match_found = false;
    }

    fn has_remote_match_context(&self) -> bool {
        self.server_kind == ServerKind::Remote
            || (self.match_found && self.server_kind != ServerKind::Local)
    }
}

impl Default for ConsolePhaseState {
    fn default() -> Self {
        Self {
            phase: ConsolePhase::Unknown,
            current_map: None,
            server_kind: ServerKind::Unknown,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            evidence: None,
            last_update_ms: 0,
        }
    }
}

struct Patterns {
    map_info: Regex,
    map_created_physics: Regex,
    change_game_state: Regex,
    server_connect: Regex,
    server_disconnect: Regex,
    lobby_created: Regex,
    lobby_destroyed: Regex,
    loop_mode_menu: Regex,
    playing_broadcast: Regex,
    app_shutdown: Regex,
    matchmaking_start: Regex,
    matchmaking_stop: Regex,
    match_found: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    map_info: regex(r#"\[Client\] Map:\s+\"([^\"]+)\""#),
    map_created_physics: regex(r"\[Client\] Created physics for\s+([^\s]+)"),
    change_game_state: regex(r"ChangeGameState:\s+(\w+)\s+\((\d+)\)"),
    server_connect: regex(r"\[Client\] CL:\s+Connected to '([^']+)'"),
    server_disconnect: regex(r"\[Client\] Disconnecting from server:\s+(\S+)"),
    lobby_created: regex(r"Lobby\s+\d+\s+for\s+Match\s+\d+\s+created"),
    lobby_destroyed: regex(r"Lobby\s+\d+\s+for\s+Match\s+\d+\s+destroyed"),
    loop_mode_menu: regex(r"LoopMode:\s*menu"),
    playing_broadcast: regex(r"Playing Broadcast"),
    app_shutdown: regex(r"Dispatching EventAppShutdown_t|Source2Shutdown"),
    matchmaking_start: regex(r"\bk_EMsgClientToGCStartMatchmaking\b"),
    matchmaking_stop: regex(r"\bk_EMsgClientToGCStopMatchmaking\b"),
    match_found: regex(r"\bk_EMsgGCToClientSDRTicket\b"),
});

static CONSOLE_PHASE_STATE: LazyLock<Mutex<ConsolePhaseState>> =
    LazyLock::new(|| Mutex::new(ConsolePhaseState::default()));

fn regex(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .expect("invalid SPLIT console phase regex")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

fn is_local_endpoint(endpoint: &str) -> bool {
    let endpoint = endpoint.trim().to_ascii_lowercase();

    if matches!(
        endpoint.as_str(),
        "loopback" | "localhost" | "127.0.0.1" | "::1"
    ) {
        return true;
    }

    if let Ok(address) = SocketAddr::from_str(&endpoint) {
        return address.ip().is_loopback();
    }

    let Some((host, port)) = endpoint.rsplit_once(':') else {
        return false;
    };

    matches!(host, "loopback" | "localhost") && port.parse::<u16>().is_ok()
}

fn apply_map(state: &mut ConsolePhaseState, map: &str) {
    let map = map.trim().to_lowercase();
    if map.is_empty() || map == "<empty>" || map == "start" {
        return;
    }

    state.current_map = Some(map.clone());
    state.last_update_ms = now_ms();

    if map == "dl_hideout" {
        state.clear_transient_match_state();
        state.set_phase(ConsolePhase::Hideout, "Map(dl_hideout)");
    } else if state.server_kind == ServerKind::Local {
        state.clear_transient_match_state();
        state.set_phase(ConsolePhase::Unknown, format!("Map({map})"));
    } else {
        state.evidence = Some(format!("Map({map})"));
    }
}

pub(crate) fn parse_line(state: &mut ConsolePhaseState, line: &str) -> bool {
    let before = state.clone();
    let patterns = &*PATTERNS;

    if patterns.matchmaking_start.is_match(line) {
        state.match_found = false;
        state.set_matchmaking(true, "GCStartMatchmaking");
    } else if patterns.matchmaking_stop.is_match(line) {
        state.set_matchmaking(false, "GCStopMatchmaking");
    } else if patterns.match_found.is_match(line) {
        state.match_found = true;
        state.set_phase(ConsolePhase::Loading, "GCMatchFound");
    } else if let Some(captures) = patterns.map_info.captures(line) {
        apply_map(state, &captures[1]);
    } else if let Some(captures) = patterns.map_created_physics.captures(line) {
        apply_map(state, &captures[1]);
    } else if patterns.playing_broadcast.is_match(line) {
        state.clear_transient_match_state();
        state.set_phase(ConsolePhase::Spectating, "PlayingBroadcast");
    } else if let Some(captures) = patterns.server_connect.captures(line) {
        state.server_kind = if is_local_endpoint(&captures[1]) {
            ServerKind::Local
        } else {
            ServerKind::Remote
        };
        state.last_update_ms = now_ms();

        if state.server_kind == ServerKind::Remote {
            state.match_found = true;
            state.current_map = None;
            state.set_phase(ConsolePhase::Loading, "ServerConnected(remote)");
        } else {
            state.clear_transient_match_state();
            if state.current_map.as_deref() == Some("dl_hideout") {
                state.set_phase(ConsolePhase::Hideout, "ServerConnected(local)");
            } else {
                state.set_phase(ConsolePhase::Unknown, "ServerConnected(local)");
            }
        }
    } else if patterns.lobby_created.is_match(line) {
        state.match_found = true;
        state.current_map = None;
        state.set_phase(ConsolePhase::Loading, "LobbyCreated");
    } else if let Some(captures) = patterns.change_game_state.captures(line) {
        let name = captures[1].to_lowercase();
        let id = captures[2].parse::<u32>().unwrap_or_default();

        if name == "matchintro" || id == 4 {
            if state.has_remote_match_context() {
                state.set_phase(ConsolePhase::Loading, "ChangeGameState(4)");
            }
        } else if name == "gameinprogress" || name == "inprogress" || id == 7 {
            if state.has_remote_match_context() {
                state.clear_transient_match_state();
                state.set_phase(ConsolePhase::InMatch, "ChangeGameState(7)");
            }
        } else if (name == "postgame" || name == "postmatch" || id == 6)
            && matches!(state.phase, ConsolePhase::InMatch | ConsolePhase::PostMatch)
        {
            state.clear_transient_match_state();
            state.set_phase(ConsolePhase::PostMatch, "ChangeGameState(6)");
        }
    } else if patterns.lobby_destroyed.is_match(line) {
        let completed_match =
            matches!(state.phase, ConsolePhase::InMatch | ConsolePhase::PostMatch);
        state.clear_transient_match_state();
        state.server_kind = ServerKind::Unknown;
        state.current_map = None;
        if completed_match {
            state.set_phase(ConsolePhase::PostMatch, "LobbyDestroyed");
        } else {
            *state = ConsolePhaseState::main_menu("LobbyDestroyed");
        }
    } else if let Some(captures) = patterns.server_disconnect.captures(line) {
        let reason = captures[1].to_uppercase();
        let completed_match = state.server_kind == ServerKind::Remote
            && matches!(
                state.phase,
                ConsolePhase::InMatch | ConsolePhase::PostMatch | ConsolePhase::Spectating
            );
        state.clear_transient_match_state();
        state.server_kind = ServerKind::Unknown;
        state.current_map = None;

        if completed_match && !reason.contains("LOOPDEACTIVATE") {
            state.set_phase(
                ConsolePhase::PostMatch,
                format!("ServerDisconnected({reason})"),
            );
        } else {
            *state = ConsolePhaseState::main_menu(&format!("ServerDisconnected({reason})"));
        }
    } else if patterns.loop_mode_menu.is_match(line) {
        *state = ConsolePhaseState::main_menu("LoopMode(menu)");
    } else if patterns.app_shutdown.is_match(line) {
        *state = ConsolePhaseState::main_menu("AppShutdown");
    }

    *state != before
}

pub(crate) fn observe_line(line: &str) {
    if let Ok(mut state) = CONSOLE_PHASE_STATE.lock() {
        parse_line(&mut state, line);
    }
}

pub(crate) fn replace_from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) {
    let state = state_from_lines(lines);

    if let Ok(mut current) = CONSOLE_PHASE_STATE.lock() {
        *current = state;
    }
}

fn state_from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> ConsolePhaseState {
    let mut state = ConsolePhaseState::main_menu("MainMenuFallback");
    for line in lines {
        parse_line(&mut state, line);
    }

    state
}

pub(crate) fn reset_for_session() {
    if let Ok(mut state) = CONSOLE_PHASE_STATE.lock() {
        *state = ConsolePhaseState::main_menu("MainMenuFallback");
    }
}

pub(crate) fn snapshot() -> ConsolePhaseState {
    CONSOLE_PHASE_STATE
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const START_MATCHMAKING: &str =
        "[GCClient] Send msg 9010 (k_EMsgClientToGCStartMatchmaking), 284 bytes";
    const STOP_MATCHMAKING: &str =
        "[GCClient] Send msg 9012 (k_EMsgClientToGCStopMatchmaking), 17 bytes";
    const MATCH_FOUND: &str = "[GCClient] Recv msg 9100 (k_EMsgGCToClientSDRTicket), 384 bytes";
    const REMOTE_CONNECT: &str = "[Client] CL: Connected to '203.0.113.8:27015'";
    const GAME_IN_PROGRESS: &str = "ChangeGameState: GameInProgress (7)";

    #[test]
    fn gc_start_and_stop_update_matchmaking_state() {
        let mut state = ConsolePhaseState::default();

        parse_line(&mut state, START_MATCHMAKING);
        assert!(state.matchmaking_active);
        assert_eq!(state.evidence.as_deref(), Some("GCStartMatchmaking"));

        parse_line(&mut state, STOP_MATCHMAKING);
        assert!(!state.matchmaking_active);
        assert!(state.matchmaking_observed);
        assert_eq!(state.evidence.as_deref(), Some("GCStopMatchmaking"));
    }

    #[test]
    fn last_gc_matchmaking_command_wins() {
        let start_then_stop = state_from_lines([START_MATCHMAKING, STOP_MATCHMAKING]);
        assert!(!start_then_stop.matchmaking_active);
        assert!(start_then_stop.matchmaking_observed);

        let stop_then_start = state_from_lines([STOP_MATCHMAKING, START_MATCHMAKING]);
        assert!(stop_then_start.matchmaking_active);
    }

    #[test]
    fn startup_history_reconstructs_matchmaking_state() {
        let active = state_from_lines([
            "unrelated startup line",
            START_MATCHMAKING,
            "another unrelated line",
        ]);
        assert!(active.matchmaking_active);

        let inactive = state_from_lines([
            START_MATCHMAKING,
            "another unrelated line",
            STOP_MATCHMAKING,
        ]);
        assert!(!inactive.matchmaking_active);
        assert!(inactive.matchmaking_observed);
    }

    #[test]
    fn gc_responses_do_not_repeat_transitions() {
        let mut state = ConsolePhaseState::default();
        assert!(parse_line(&mut state, START_MATCHMAKING));
        assert!(!parse_line(
            &mut state,
            "[GCClient] Recv msg 9011 (k_EMsgClientToGCStartMatchmakingResponse), 19 bytes",
        ));

        assert!(parse_line(&mut state, STOP_MATCHMAKING));
        assert!(!parse_line(
            &mut state,
            "[GCClient] Recv msg 9013 (k_EMsgClientToGCStopMatchmakingResponse), 19 bytes",
        ));
    }

    #[test]
    fn match_found_enters_loading_and_survives_stop_matchmaking() {
        let mut state = ConsolePhaseState::default();
        parse_line(&mut state, START_MATCHMAKING);
        parse_line(&mut state, MATCH_FOUND);

        assert_eq!(state.phase, ConsolePhase::Loading);
        assert!(state.match_found);
        assert_eq!(state.evidence.as_deref(), Some("GCMatchFound"));

        parse_line(&mut state, STOP_MATCHMAKING);
        assert_eq!(state.phase, ConsolePhase::Loading);
        assert!(state.match_found);
        assert!(!state.matchmaking_active);
    }

    #[test]
    fn remote_match_context_and_game_in_progress_enter_in_match() {
        let state = state_from_lines([
            START_MATCHMAKING,
            MATCH_FOUND,
            REMOTE_CONNECT,
            GAME_IN_PROGRESS,
        ]);

        assert_eq!(state.phase, ConsolePhase::InMatch);
        assert_eq!(state.server_kind, ServerKind::Remote);
        assert!(!state.match_found);
        assert!(!state.matchmaking_active);
    }

    #[test]
    fn startup_replay_reconstructs_loading_and_in_match() {
        let loading = state_from_lines([START_MATCHMAKING, MATCH_FOUND]);
        assert_eq!(loading.phase, ConsolePhase::Loading);
        assert!(loading.match_found);

        let in_match = state_from_lines([START_MATCHMAKING, MATCH_FOUND, GAME_IN_PROGRESS]);
        assert_eq!(in_match.phase, ConsolePhase::InMatch);
    }

    #[test]
    fn classifies_only_real_local_endpoints_as_local() {
        for endpoint in [
            "loopback",
            "loopback:1",
            "localhost",
            "localhost:27015",
            "127.0.0.1",
            "127.0.0.1:27015",
            "::1",
            "[::1]:27015",
        ] {
            assert!(is_local_endpoint(endpoint), "{endpoint} should be local");
        }

        for endpoint in [
            "192.168.1.20:27015",
            "203.0.113.8:27015",
            "127.example:27015",
        ] {
            assert!(!is_local_endpoint(endpoint), "{endpoint} should be remote");
        }
    }

    #[test]
    fn connected_localhost_and_external_endpoints_set_server_kind() {
        let mut state = ConsolePhaseState::default();
        parse_line(&mut state, "[Client] CL: Connected to '127.0.0.1:27015'");
        assert_eq!(state.server_kind, ServerKind::Local);

        parse_line(&mut state, "[Client] CL: Connected to '203.0.113.8:27015'");
        assert_eq!(state.server_kind, ServerKind::Remote);
    }

    #[test]
    fn hideout_map_has_priority_over_internal_game_states() {
        let mut state = ConsolePhaseState::default();
        parse_line(&mut state, "[Client] Map: \"dl_hideout\"");
        parse_line(&mut state, "ChangeGameState: GameInProgress (7)");

        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert_eq!(state.current_map.as_deref(), Some("dl_hideout"));
    }

    #[test]
    fn local_midtown_game_in_progress_stays_local_context() {
        let state = state_from_lines([
            "[Client] CL: Connected to '127.0.0.1:27015'",
            "[Client] Map: \"dl_midtown\"",
            GAME_IN_PROGRESS,
        ]);

        assert_eq!(state.phase, ConsolePhase::Unknown);
        assert_eq!(state.current_map.as_deref(), Some("dl_midtown"));
        assert_eq!(state.server_kind, ServerKind::Local);
    }

    #[test]
    fn recognizes_confirmed_match_lifecycle_states() {
        let mut state = state_from_lines([MATCH_FOUND, "ChangeGameState: MatchIntro (4)"]);
        assert_eq!(state.phase, ConsolePhase::Loading);

        parse_line(&mut state, GAME_IN_PROGRESS);
        assert_eq!(state.phase, ConsolePhase::InMatch);

        parse_line(&mut state, "ChangeGameState: PostGame (6)");
        assert_eq!(state.phase, ConsolePhase::PostMatch);
        assert!(!state.match_found);
    }

    #[test]
    fn match_transitions_reset_on_hideout_and_menu() {
        let mut state = state_from_lines([MATCH_FOUND, GAME_IN_PROGRESS]);
        parse_line(&mut state, "[Client] Map: \"dl_hideout\"");
        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert!(!state.match_found);

        let mut state = state_from_lines([MATCH_FOUND, GAME_IN_PROGRESS]);
        parse_line(&mut state, "LoopMode: menu");
        assert_eq!(state.phase, ConsolePhase::MainMenu);
        assert_eq!(state.current_map, None);
        assert!(!state.match_found);
        assert!(!state.matchmaking_active);
    }

    #[test]
    fn recognizes_spectating_and_return_to_hideout() {
        let mut state = ConsolePhaseState::default();
        parse_line(&mut state, "[HostStateManager] Playing Broadcast");
        assert_eq!(state.phase, ConsolePhase::Spectating);

        parse_line(&mut state, "[Client] Created physics for dl_hideout");
        assert_eq!(state.phase, ConsolePhase::Hideout);
    }

    #[test]
    fn records_special_maps_without_guessing_their_phase() {
        let mut state = ConsolePhaseState::default();
        parse_line(&mut state, "[Client] Map: \"dl_midtown\"");
        assert_eq!(state.current_map.as_deref(), Some("dl_midtown"));
        assert_eq!(state.phase, ConsolePhase::Unknown);

        parse_line(&mut state, "[Client] Map: \"new_player_basics\"");
        assert_eq!(state.current_map.as_deref(), Some("new_player_basics"));
    }

    #[test]
    fn ignores_irrelevant_lines() {
        let mut state = ConsolePhaseState::default();
        assert!(!parse_line(
            &mut state,
            "getpos_exact returned setpos 1 2 3"
        ));
        assert_eq!(state, ConsolePhaseState::default());
    }
}
