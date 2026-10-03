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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeroCaptureMode {
    Disabled,
    FreeSwap,
    FirstMatchHero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeroSignalOutcome {
    accepted: bool,
    reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeroSignalSource {
    LoadedHero,
    ClientHeroVmdl,
}

impl HeroSignalSource {
    const fn label(self) -> &'static str {
        match self {
            Self::LoadedHero => "loaded_hero",
            Self::ClientHeroVmdl => "client_hero_vmdl",
        }
    }
}

struct HeroSignal {
    source: HeroSignalSource,
    raw: String,
    normalized: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsolePhaseState {
    pub phase: ConsolePhase,
    pub current_map: Option<String>,
    pub server_kind: ServerKind,
    pub broadcast_active: bool,
    pub spectating_match_id: Option<u64>,
    pub matchmaking_active: bool,
    pub matchmaking_observed: bool,
    pub match_found: bool,
    pub current_hero: Option<String>,
    pub(crate) hero_capture_mode: HeroCaptureMode,
    pub evidence: Option<String>,
    pub last_update_ms: u64,
}

impl ConsolePhaseState {
    fn main_menu(evidence: &str) -> Self {
        Self {
            phase: ConsolePhase::MainMenu,
            current_map: None,
            server_kind: ServerKind::Unknown,
            broadcast_active: false,
            spectating_match_id: None,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            current_hero: None,
            hero_capture_mode: HeroCaptureMode::Disabled,
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

    fn enter_broadcast(&mut self, match_id: Option<u64>, evidence: &str) {
        let was_active = self.broadcast_active;
        self.clear_transient_match_state();
        self.clear_hero();
        self.server_kind = ServerKind::Unknown;
        self.broadcast_active = true;
        if !was_active || match_id.is_some() {
            self.spectating_match_id = match_id;
        }
        self.set_phase(ConsolePhase::Spectating, evidence);
    }

    fn clear_broadcast(&mut self) {
        self.broadcast_active = false;
        self.spectating_match_id = None;
    }

    fn clear_hero(&mut self) {
        if self.current_map.as_deref() == Some("dl_hideout")
            && (self.current_hero.is_some() || self.hero_capture_mode != HeroCaptureMode::Disabled)
        {
            log_hideout_tracking(false, "reset");
        }
        self.current_hero = None;
        self.hero_capture_mode = HeroCaptureMode::Disabled;
    }

    fn begin_real_match_hero_capture(&mut self) {
        if self.current_map.as_deref() == Some("dl_hideout") {
            log_hideout_tracking(false, "real_match");
        }
        if !self
            .current_hero
            .as_deref()
            .is_some_and(crate::discord::is_known_hero_key)
        {
            self.current_hero = None;
        }
        self.hero_capture_mode = HeroCaptureMode::FirstMatchHero;
    }

    fn hero_window_open(&self) -> bool {
        self.hero_capture_mode != HeroCaptureMode::Disabled
    }

    fn accept_hero(&mut self, hero: String, source: HeroSignalSource) -> HeroSignalOutcome {
        if self.broadcast_active {
            return HeroSignalOutcome {
                accepted: false,
                reason: "broadcast_active",
            };
        }

        if matches!(
            self.phase,
            ConsolePhase::MainMenu | ConsolePhase::PostMatch | ConsolePhase::Spectating
        ) {
            return HeroSignalOutcome {
                accepted: false,
                reason: "protected_phase",
            };
        }

        if source == HeroSignalSource::ClientHeroVmdl
            && self
                .current_hero
                .as_deref()
                .is_some_and(crate::discord::is_known_hero_key)
            && !crate::discord::is_known_hero_key(&hero)
        {
            return HeroSignalOutcome {
                accepted: false,
                reason: "unknown_vmdl_after_known_hero",
            };
        }

        match self.hero_capture_mode {
            HeroCaptureMode::Disabled => HeroSignalOutcome {
                accepted: false,
                reason: "window_closed",
            },
            HeroCaptureMode::FreeSwap => {
                if self.current_hero.as_deref() == Some(hero.as_str()) {
                    HeroSignalOutcome {
                        accepted: false,
                        reason: "unchanged",
                    }
                } else {
                    self.current_hero = Some(hero);
                    self.last_update_ms = now_ms();
                    HeroSignalOutcome {
                        accepted: true,
                        reason: "free_swap",
                    }
                }
            }
            HeroCaptureMode::FirstMatchHero => {
                self.current_hero = Some(hero);
                self.hero_capture_mode = HeroCaptureMode::Disabled;
                self.last_update_ms = now_ms();
                HeroSignalOutcome {
                    accepted: true,
                    reason: "first_match_hero",
                }
            }
        }
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
            broadcast_active: false,
            spectating_match_id: None,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            current_hero: None,
            hero_capture_mode: HeroCaptureMode::Disabled,
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
    spectate_lobby: Regex,
    hltv_broadcast_start: Regex,
    broadcast_url: Regex,
    app_shutdown: Regex,
    matchmaking_start: Regex,
    matchmaking_stop: Regex,
    match_found: Regex,
    loaded_hero: Regex,
    client_hero_vmdl: Regex,
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
    spectate_lobby: regex(r"\bk_EMsgClientToGCSpectateLobby\b"),
    hltv_broadcast_start: regex(
        r"\[HLTV Broadcast\].*(?:Broadcast:\s*Synchronizing stream|Stream State\s+Stop\s*->\s*Sync)",
    ),
    broadcast_url: regex(r"(?:^|https?://[^/\s)]+)?/tv/([0-9]+)_([0-9]+)(?:$|[/?#\s),])"),
    app_shutdown: regex(r"Dispatching EventAppShutdown_t|Source2Shutdown"),
    matchmaking_start: regex(r"\bk_EMsgClientToGCStartMatchmaking\b"),
    matchmaking_stop: regex(r"\bk_EMsgClientToGCStopMatchmaking\b"),
    match_found: regex(r"\bk_EMsgGCToClientSDRTicket\b"),
    loaded_hero: regex(r"\[Server\]\s+Loaded hero\s+\d+/(hero_\w+)"),
    client_hero_vmdl: regex(r"VMDL Camera Pose Success!.*models/heroes(?:_wip|_staging)?/(\w+)/"),
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

#[cfg(debug_assertions)]
fn log_hero_signal(
    state: &ConsolePhaseState,
    signal: &HeroSignal,
    window_open: bool,
    outcome: HeroSignalOutcome,
) {
    println!(
        "[SPLIT][Hero] source={} raw={} normalized={} phase={:?} window_open={} accepted={} reason={}",
        signal.source.label(),
        signal.raw,
        signal.normalized.as_deref().unwrap_or("<invalid>"),
        state.phase,
        window_open,
        outcome.accepted,
        outcome.reason
    );
}

#[cfg(not(debug_assertions))]
fn log_hero_signal(
    _state: &ConsolePhaseState,
    _signal: &HeroSignal,
    _window_open: bool,
    _outcome: HeroSignalOutcome,
) {
}

#[cfg(debug_assertions)]
fn log_hideout_tracking(open: bool, reason: &str) {
    println!(
        "[SPLIT][Hero] hideout_ready={} tracking_open={} reason={reason}",
        open, open
    );
}

#[cfg(not(debug_assertions))]
fn log_hideout_tracking(_open: bool, _reason: &str) {}

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

fn hero_signal(line: &str) -> Option<HeroSignal> {
    let patterns = &*PATTERNS;
    if let Some(captures) = patterns.loaded_hero.captures(line) {
        let raw = captures[1].to_string();
        return Some(HeroSignal {
            source: HeroSignalSource::LoadedHero,
            normalized: crate::discord::canonicalize_hero_key(&raw),
            raw,
        });
    }
    if let Some(captures) = patterns.client_hero_vmdl.captures(line) {
        let raw = captures[1].to_string();
        return Some(HeroSignal {
            source: HeroSignalSource::ClientHeroVmdl,
            normalized: crate::discord::canonicalize_hero_key(&raw),
            raw,
        });
    }
    None
}

fn broadcast_match_id(line: &str) -> Option<u64> {
    PATTERNS
        .broadcast_url
        .captures(line)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<u64>().ok())
}

fn apply_hero_signal(state: &mut ConsolePhaseState, signal: HeroSignal) {
    let window_open = state.hero_window_open();
    let outcome = match signal.normalized.clone() {
        Some(hero) => state.accept_hero(hero, signal.source),
        None => HeroSignalOutcome {
            accepted: false,
            reason: "invalid_key",
        },
    };
    log_hero_signal(state, &signal, window_open, outcome);
}

fn apply_map(state: &mut ConsolePhaseState, map: &str, physics_loaded: bool) {
    let map = map.trim().to_lowercase();
    if map.is_empty() || map == "<empty>" || map == "start" {
        return;
    }

    let entering_map = state.current_map.as_deref() != Some(map.as_str());
    state.current_map = Some(map.clone());
    state.last_update_ms = now_ms();

    if map == "dl_hideout" {
        state.clear_broadcast();
        state.clear_transient_match_state();
        if entering_map {
            state.clear_hero();
        }
        if physics_loaded {
            state.hero_capture_mode = HeroCaptureMode::FreeSwap;
            log_hideout_tracking(true, "created_physics");
        }
        state.set_phase(ConsolePhase::Hideout, "Map(dl_hideout)");
    } else if state.broadcast_active {
        state.clear_transient_match_state();
        state.set_phase(ConsolePhase::Spectating, format!("BroadcastMap({map})"));
    } else if state.server_kind == ServerKind::Local {
        state.clear_transient_match_state();
        if entering_map {
            state.clear_hero();
            if map == "dl_midtown" {
                super::district::notify_midtown_entered();
            }
        }
        if matches!(map.as_str(), "dl_midtown" | "new_player_basics") {
            state.hero_capture_mode = HeroCaptureMode::FreeSwap;
        }
        state.set_phase(ConsolePhase::Unknown, format!("Map({map})"));
    } else {
        state.evidence = Some(format!("Map({map})"));

        if physics_loaded
            && map == "dl_midtown"
            && state.server_kind == ServerKind::Remote
            && state.has_remote_match_context()
            && matches!(state.phase, ConsolePhase::Loading | ConsolePhase::InMatch)
        {
            state.clear_transient_match_state();
            state.set_phase(ConsolePhase::InMatch, "RemotePhysics(dl_midtown)");
        }
    }
}

pub(crate) fn parse_line(state: &mut ConsolePhaseState, line: &str) -> bool {
    let before = state.clone();
    let patterns = &*PATTERNS;

    if patterns.spectate_lobby.is_match(line) {
        state.clear_broadcast();
        state.enter_broadcast(None, "GCSpectateLobby");
    } else if patterns.playing_broadcast.is_match(line) {
        let match_id = broadcast_match_id(line);
        state.enter_broadcast(match_id, "PlayingBroadcast");
    } else if patterns.hltv_broadcast_start.is_match(line) {
        state.enter_broadcast(None, "HLTVBroadcast");
    } else if patterns.matchmaking_start.is_match(line) {
        state.clear_broadcast();
        state.match_found = false;
        state.set_matchmaking(true, "GCStartMatchmaking");
    } else if patterns.matchmaking_stop.is_match(line) {
        state.set_matchmaking(false, "GCStopMatchmaking");
    } else if patterns.match_found.is_match(line) {
        state.clear_broadcast();
        if !state.match_found
            || !matches!(state.phase, ConsolePhase::Loading | ConsolePhase::InMatch)
        {
            state.begin_real_match_hero_capture();
        }
        state.match_found = true;
        state.set_phase(ConsolePhase::Loading, "GCMatchFound");
    } else if let Some(captures) = patterns.map_info.captures(line) {
        apply_map(state, &captures[1], false);
    } else if let Some(captures) = patterns.map_created_physics.captures(line) {
        apply_map(state, &captures[1], true);
    } else if let Some(captures) = patterns.server_connect.captures(line) {
        state.clear_broadcast();
        state.server_kind = if is_local_endpoint(&captures[1]) {
            ServerKind::Local
        } else {
            ServerKind::Remote
        };
        state.last_update_ms = now_ms();

        if state.server_kind == ServerKind::Remote {
            if !matches!(state.phase, ConsolePhase::Loading | ConsolePhase::InMatch) {
                state.begin_real_match_hero_capture();
            }
            state.match_found = true;
            state.current_map = None;
            state.set_phase(ConsolePhase::Loading, "ServerConnected(remote)");
        } else {
            state.clear_transient_match_state();
            state.clear_hero();
            if state.current_map.as_deref() == Some("dl_hideout") {
                state.set_phase(ConsolePhase::Hideout, "ServerConnected(local)");
            } else {
                state.set_phase(ConsolePhase::Unknown, "ServerConnected(local)");
            }
        }
    } else if patterns.lobby_created.is_match(line) {
        state.clear_broadcast();
        if !matches!(state.phase, ConsolePhase::Loading | ConsolePhase::InMatch) {
            state.begin_real_match_hero_capture();
        }
        state.match_found = true;
        state.current_map = None;
        state.set_phase(ConsolePhase::Loading, "LobbyCreated");
    } else if let Some(captures) = patterns.change_game_state.captures(line) {
        let name = captures[1].to_lowercase();
        let id = captures[2].parse::<u32>().unwrap_or_default();

        if state.broadcast_active {
            state.set_phase(
                ConsolePhase::Spectating,
                format!("BroadcastGameState({id})"),
            );
        } else if name == "matchintro" || id == 4 {
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
            state.clear_hero();
            state.set_phase(ConsolePhase::PostMatch, "ChangeGameState(6)");
        }
    } else if patterns.lobby_destroyed.is_match(line) {
        let completed_match =
            matches!(state.phase, ConsolePhase::InMatch | ConsolePhase::PostMatch);
        state.clear_transient_match_state();
        state.clear_hero();
        state.server_kind = ServerKind::Unknown;
        state.current_map = None;
        if completed_match {
            state.set_phase(ConsolePhase::PostMatch, "LobbyDestroyed");
        } else {
            *state = ConsolePhaseState::main_menu("LobbyDestroyed");
        }
    } else if let Some(captures) = patterns.server_disconnect.captures(line) {
        let reason = captures[1].to_uppercase();
        let confirmed_hideout = state.phase == ConsolePhase::Hideout
            || state.current_map.as_deref() == Some("dl_hideout");
        let completed_match = state.server_kind == ServerKind::Remote
            && matches!(
                state.phase,
                ConsolePhase::InMatch | ConsolePhase::PostMatch | ConsolePhase::Spectating
            );
        if confirmed_hideout && reason.contains("LOOPDEACTIVATE") {
            state.clear_transient_match_state();
            state.set_phase(ConsolePhase::Hideout, "Map(dl_hideout)");
        } else {
            state.clear_transient_match_state();
            state.clear_hero();
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
        }
    } else if patterns.loop_mode_menu.is_match(line) {
        *state = ConsolePhaseState::main_menu("LoopMode(menu)");
    } else if patterns.app_shutdown.is_match(line) {
        *state = ConsolePhaseState::main_menu("AppShutdown");
    } else if let Some(signal) = hero_signal(line) {
        apply_hero_signal(state, signal);
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
    let mut pending_hideout_hero = None;
    for line in lines {
        let replay_hero = hero_signal(line).and_then(|signal| signal.normalized);
        parse_line(&mut state, line);

        let safe_hideout_context = state.phase == ConsolePhase::Hideout
            && state.current_map.as_deref() == Some("dl_hideout");
        if !safe_hideout_context {
            pending_hideout_hero = None;
        } else if state.current_hero.is_none()
            && state.hero_capture_mode == HeroCaptureMode::Disabled
            && replay_hero.is_some()
        {
            pending_hideout_hero = replay_hero;
        }
    }

    if state.phase == ConsolePhase::Hideout
        && state.current_map.as_deref() == Some("dl_hideout")
        && state.current_hero.is_none()
    {
        if let Some(hero) = pending_hideout_hero {
            state.current_hero = Some(hero.clone());
            state.hero_capture_mode = HeroCaptureMode::FreeSwap;
            state.last_update_ms = now_ms();
            log_startup_hero_recovery(&hero);
        }
    }

    state
}

#[cfg(test)]
pub(crate) fn state_from_test_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> ConsolePhaseState {
    state_from_lines(lines)
}

#[cfg(debug_assertions)]
fn log_startup_hero_recovery(hero: &str) {
    println!(
        "[SPLIT][Hero] startup_replay_recovered=true hero={hero} phase=Hideout tracking_open=true"
    );
}

#[cfg(not(debug_assertions))]
fn log_startup_hero_recovery(_hero: &str) {}

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
    const HIDEOUT_LOADED: &str = "[Client] Created physics for dl_hideout";
    const BOOKWORM_LOADED: &str = "[Server] Loaded hero 1/hero_bookworm";
    const BOOKWORM_VMDL: &str = "VMDL Camera Pose Success! models/heroes/bookworm/hero.vmdl";
    const SPECTATE_LOBBY: &str =
        "[GCClient] Send msg 9109 (k_EMsgClientToGCSpectateLobby), 24 bytes";
    const PLAYING_BROADCAST: &str = "[HostStateManager] CHostStateMgr::QueueNewRequest( Playing Broadcast (http://dist1-ord1.steamcontent.com/tv/110501755_856385445), 3 )";

    #[test]
    fn parses_only_the_match_component_of_a_broadcast_tv_url() {
        assert_eq!(
            broadcast_match_id("/tv/110501755_856385445"),
            Some(110501755)
        );
        assert_eq!(
            broadcast_match_id(
                "request 77: http://dist1-ord1.steamcontent.com/tv/110501755_856385445, sequence 99"
            ),
            Some(110501755)
        );
        assert_eq!(
            broadcast_match_id("https://example.com/watch/110501755"),
            None
        );
        assert_eq!(broadcast_match_id("/tv/abc_123"), None);
        assert_eq!(broadcast_match_id("/tv/110501755"), None);
    }

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
    fn local_midtown_reopens_hero_capture_for_runtime_loaded_hero_signal() {
        let state = state_from_lines([
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Map: \"dl_midtown\"",
            "[Client] Created physics for dl_midtown",
            "[Server] Loaded hero 2862/hero_ratking",
        ]);

        assert_eq!(state.phase, ConsolePhase::Unknown);
        assert_eq!(state.current_map.as_deref(), Some("dl_midtown"));
        assert_eq!(state.server_kind, ServerKind::Local);
        assert_eq!(state.current_hero.as_deref(), Some("ratking"));
        assert_eq!(state.hero_capture_mode, HeroCaptureMode::FreeSwap);
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
        assert!(state.broadcast_active);

        parse_line(&mut state, "[Client] Created physics for dl_hideout");
        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert!(!state.broadcast_active);
        assert_eq!(state.spectating_match_id, None);
    }

    #[test]
    fn hltv_context_survives_midtown_game_state_and_broadcast_hero_noise() {
        let state = state_from_lines([
            SPECTATE_LOBBY,
            "[HLTV Broadcast] Broadcast: Synchronizing stream",
            PLAYING_BROADCAST,
            "[Client] Created physics for dl_midtown",
            "OnGameStateChanged: GameInProgress (7)",
            GAME_IN_PROGRESS,
            BOOKWORM_LOADED,
            BOOKWORM_VMDL,
        ]);

        assert_eq!(state.phase, ConsolePhase::Spectating);
        assert!(state.broadcast_active);
        assert_eq!(state.spectating_match_id, Some(110501755));
        assert_eq!(state.current_map.as_deref(), Some("dl_midtown"));
        assert_eq!(state.current_hero, None);
        assert_eq!(state.hero_capture_mode, HeroCaptureMode::Disabled);
    }

    #[test]
    fn broadcast_match_id_persists_until_a_strong_session_exit() {
        let mut state = state_from_lines([PLAYING_BROADCAST, GAME_IN_PROGRESS]);
        assert_eq!(state.spectating_match_id, Some(110501755));

        parse_line(&mut state, "LoopMode: menu");
        assert_eq!(state.phase, ConsolePhase::MainMenu);
        assert!(!state.broadcast_active);
        assert_eq!(state.spectating_match_id, None);

        let local = state_from_lines([
            PLAYING_BROADCAST,
            "[Client] CL: Connected to 'loopback:1'",
            "[Client] Created physics for dl_midtown",
        ]);
        assert!(!local.broadcast_active);
        assert_eq!(local.spectating_match_id, None);
        assert_eq!(local.server_kind, ServerKind::Local);
        assert_eq!(local.phase, ConsolePhase::Unknown);

        let remote = state_from_lines([PLAYING_BROADCAST, MATCH_FOUND, GAME_IN_PROGRESS]);
        assert!(!remote.broadcast_active);
        assert_eq!(remote.spectating_match_id, None);
        assert_eq!(remote.phase, ConsolePhase::InMatch);
    }

    #[test]
    fn parses_and_normalizes_loaded_hero() {
        let state = state_from_lines([HIDEOUT_LOADED, BOOKWORM_LOADED]);
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));
        assert_eq!(
            crate::discord::canonicalize_hero_key("hero_Bookworm").as_deref(),
            Some("bookworm")
        );
    }

    #[test]
    fn parses_case_insensitive_vmdl_hero_paths() {
        let state = state_from_lines([
            HIDEOUT_LOADED,
            "vmdl camera pose success! MODELS/HEROES_WIP/Bookworm/hero.vmdl",
        ]);
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));

        let staging = state_from_lines([
            HIDEOUT_LOADED,
            "VMDL Camera Pose Success! models/heroes_staging/HERO_Bookworm/hero.vmdl",
        ]);
        assert_eq!(staging.current_hero.as_deref(), Some("bookworm"));
    }

    #[test]
    fn menu_and_spectating_ignore_hero_signals() {
        let menu = state_from_lines([BOOKWORM_LOADED, BOOKWORM_VMDL]);
        assert_eq!(menu.phase, ConsolePhase::MainMenu);
        assert_eq!(menu.current_hero, None);

        let spectating = state_from_lines([
            HIDEOUT_LOADED,
            "[HostStateManager] Playing Broadcast",
            BOOKWORM_LOADED,
        ]);
        assert_eq!(spectating.phase, ConsolePhase::Spectating);
        assert_eq!(spectating.current_hero, None);
    }

    #[test]
    fn hideout_waits_for_physics_then_tracks_free_swaps() {
        let mut state = ConsolePhaseState::main_menu("test");
        parse_line(&mut state, "[Client] Map: \"dl_hideout\"");
        parse_line(&mut state, BOOKWORM_LOADED);
        assert_eq!(state.current_hero, None);

        parse_line(&mut state, HIDEOUT_LOADED);
        parse_line(&mut state, BOOKWORM_LOADED);
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));

        parse_line(&mut state, "[Server] Loaded hero 2/hero_infernus");
        assert_eq!(state.current_hero.as_deref(), Some("infernus"));
    }

    #[test]
    fn startup_replay_recovers_hideout_hero_when_physics_precedes_tail() {
        let state = state_from_lines([
            "[Client] Map: \"dl_hideout\"",
            "unrelated line after the omitted physics event",
            "[Server] Loaded hero 7/hero_gigawatt",
        ]);

        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert_eq!(state.current_hero.as_deref(), Some("gigawatt"));
        assert_eq!(state.hero_capture_mode, HeroCaptureMode::FreeSwap);
    }

    #[test]
    fn real_match_preserves_known_hero_and_locks_first_match_update() {
        let mut state = state_from_lines([HIDEOUT_LOADED, BOOKWORM_LOADED]);
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));

        parse_line(&mut state, MATCH_FOUND);
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));
        assert_eq!(state.hero_capture_mode, HeroCaptureMode::FirstMatchHero);

        parse_line(&mut state, "[Server] Loaded hero 3/hero_infernus");
        assert_eq!(state.current_hero.as_deref(), Some("infernus"));

        parse_line(&mut state, BOOKWORM_VMDL);
        assert_eq!(state.current_hero.as_deref(), Some("infernus"));
    }

    #[test]
    fn stale_loop_deactivate_cannot_overwrite_confirmed_hideout() {
        let mut state = state_from_lines([
            MATCH_FOUND,
            REMOTE_CONNECT,
            "[Server] Loaded hero 3/hero_ratking",
            GAME_IN_PROGRESS,
            "ChangeGameState: PostGame (6)",
            HIDEOUT_LOADED,
            "[Server] Loaded hero 4/hero_ratking",
        ]);
        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert_eq!(state.current_map.as_deref(), Some("dl_hideout"));
        assert_eq!(state.current_hero.as_deref(), Some("ratking"));

        parse_line(
            &mut state,
            "[Client] Disconnecting from server: NETWORK_DISCONNECT_LOOPDEACTIVATE",
        );

        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert_eq!(state.current_map.as_deref(), Some("dl_hideout"));
        assert_eq!(state.current_hero.as_deref(), Some("ratking"));
        assert_eq!(state.evidence.as_deref(), Some("Map(dl_hideout)"));
    }

    #[test]
    fn hideout_vmdl_aliases_update_but_unknowns_and_current_variants_do_not() {
        let mut state = ConsolePhaseState::main_menu("test");
        parse_line(&mut state, HIDEOUT_LOADED);
        parse_line(&mut state, "[Server] Loaded hero 1/hero_gigawatt");
        assert_eq!(state.current_hero.as_deref(), Some("gigawatt"));

        assert!(!parse_line(
            &mut state,
            "VMDL Camera Pose Success! models/heroes/gigawatt_prisoner/hero.vmdl"
        ));
        assert_eq!(state.current_hero.as_deref(), Some("gigawatt"));

        assert!(parse_line(&mut state, BOOKWORM_VMDL));
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));

        assert!(!parse_line(
            &mut state,
            "VMDL Camera Pose Success! models/heroes/totally_unknown_model/hero.vmdl"
        ));
        assert_eq!(state.current_hero.as_deref(), Some("bookworm"));

        assert!(parse_line(
            &mut state,
            "[Server] Loaded hero 2/hero_inferno"
        ));
        assert_eq!(state.current_hero.as_deref(), Some("inferno"));
    }

    #[test]
    fn sandbox_tracks_free_swaps_and_hideout_reopens_tracking() {
        let mut state = state_from_lines([
            "[Client] CL: Connected to '127.0.0.1:27015'",
            "[Client] Map: \"new_player_basics\"",
            BOOKWORM_LOADED,
            "[Server] Loaded hero 4/hero_infernus",
        ]);
        assert_eq!(state.current_hero.as_deref(), Some("infernus"));

        parse_line(&mut state, MATCH_FOUND);
        parse_line(&mut state, BOOKWORM_LOADED);
        parse_line(&mut state, HIDEOUT_LOADED);
        parse_line(&mut state, "[Server] Loaded hero 5/hero_ivy");
        assert_eq!(state.phase, ConsolePhase::Hideout);
        assert_eq!(state.current_hero.as_deref(), Some("ivy"));
    }

    #[test]
    fn startup_replay_drops_old_match_hero_after_menu_reset() {
        let state = state_from_lines([
            START_MATCHMAKING,
            MATCH_FOUND,
            BOOKWORM_LOADED,
            GAME_IN_PROGRESS,
            "LoopMode: menu",
            "[Server] Loaded hero 6/hero_infernus",
        ]);

        assert_eq!(state.phase, ConsolePhase::MainMenu);
        assert_eq!(state.current_hero, None);
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
