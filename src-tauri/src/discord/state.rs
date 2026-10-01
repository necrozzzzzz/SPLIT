use crate::deadlock::{
    console_phase::{ConsolePhase, ConsolePhaseState, ServerKind},
    deadlock_state::{DeadlockActivity, DeadlockState, GameMode, MatchMode},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedPhase {
    MainMenu,
    Matchmaking,
    Hideout,
    ExploreNyc,
    Sandbox,
    Loading,
    InMatch,
    PostMatch,
    Spectating,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresenceSnapshot {
    pub activity: DeadlockActivity,
    pub match_mode: MatchMode,
    pub game_mode: GameMode,
    pub resolved_phase: ResolvedPhase,
    pub evidence: String,
    pub details: &'static str,
    pub state: &'static str,
    pub party_size: u32,
    pub party_max: u32,
    pub started_at: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct PresenceModel {
    timed_phase: Option<ResolvedPhase>,
    started_at: Option<i64>,
}

impl PresenceModel {
    pub(crate) fn observe_with_console(
        &mut self,
        enabled: bool,
        state: Option<DeadlockState>,
        console: &ConsolePhaseState,
        now: i64,
    ) -> Option<PresenceSnapshot> {
        if !enabled {
            self.reset();
            return None;
        }

        let state = state.unwrap_or_else(console_only_fallback_state);

        let (resolved_phase, evidence) = resolve_phase(state, console);
        let timed_phase = match resolved_phase {
            ResolvedPhase::Matchmaking
            | ResolvedPhase::Loading
            | ResolvedPhase::InMatch
            | ResolvedPhase::ExploreNyc
            | ResolvedPhase::Sandbox => Some(resolved_phase),
            ResolvedPhase::MainMenu
            | ResolvedPhase::Hideout
            | ResolvedPhase::PostMatch
            | ResolvedPhase::Spectating => None,
        };

        if timed_phase != self.timed_phase {
            self.timed_phase = timed_phase;
            self.started_at = timed_phase.map(|_| now);
        }

        let (details, presence_state) = match resolved_phase {
            ResolvedPhase::MainMenu => ("Deadlock", "In Menu"),
            ResolvedPhase::Matchmaking => ("Searching for Match", "Matchmaking"),
            ResolvedPhase::Hideout => ("Deadlock", "Hideout"),
            ResolvedPhase::ExploreNyc => ("Exploring NYC", "Explore NYC"),
            ResolvedPhase::Sandbox => ("Practice", "Sandbox"),
            ResolvedPhase::Loading => ("Loading into Match", "Deadlock"),
            ResolvedPhase::InMatch => ("In Match", "Deadlock"),
            ResolvedPhase::PostMatch => ("Post Match", "Deadlock"),
            ResolvedPhase::Spectating => ("Spectating", "Deadlock"),
        };

        Some(PresenceSnapshot {
            activity: state.activity,
            match_mode: state.match_mode,
            game_mode: state.game_mode,
            resolved_phase,
            evidence,
            details,
            state: presence_state,
            party_size: normalize_party_size(state.party_size, state.party_max),
            party_max: state.party_max.max(1),
            started_at: self.started_at,
        })
    }

    fn reset(&mut self) {
        self.timed_phase = None;
        self.started_at = None;
    }
}

const fn console_only_fallback_state() -> DeadlockState {
    DeadlockState {
        activity: DeadlockActivity::Idle,
        match_mode: MatchMode::Invalid,
        game_mode: GameMode::Invalid,
        party_size: 1,
        party_max: 6,
    }
}

fn resolve_phase(state: DeadlockState, console: &ConsolePhaseState) -> (ResolvedPhase, String) {
    let console_evidence = || {
        console
            .evidence
            .clone()
            .unwrap_or_else(|| "ConsolePhase".to_string())
    };

    let local_or_special_map = matches!(
        console.current_map.as_deref(),
        Some("dl_hideout") | Some("new_player_basics")
    ) || (console.current_map.as_deref() == Some("dl_midtown")
        && console.server_kind == ServerKind::Local);

    match console.phase {
        ConsolePhase::Spectating => {
            return (ResolvedPhase::Spectating, console_evidence());
        }
        ConsolePhase::InMatch if !local_or_special_map => {
            return (ResolvedPhase::InMatch, console_evidence());
        }
        ConsolePhase::PostMatch => {
            return (ResolvedPhase::PostMatch, console_evidence());
        }
        ConsolePhase::Loading => {
            return (ResolvedPhase::Loading, console_evidence());
        }
        ConsolePhase::InMatch
        | ConsolePhase::MainMenu
        | ConsolePhase::Hideout
        | ConsolePhase::Unknown => {}
    }

    if console.match_found {
        return (ResolvedPhase::Loading, "GCMatchFound".to_string());
    }

    if console.matchmaking_active {
        return (ResolvedPhase::Matchmaking, "GCStartMatchmaking".to_string());
    }

    match console.current_map.as_deref() {
        Some("dl_hideout") => {
            return (ResolvedPhase::Hideout, "Map(dl_hideout)".to_string());
        }
        Some("dl_midtown") if console.server_kind == ServerKind::Local => {
            return (
                ResolvedPhase::ExploreNyc,
                "Map(dl_midtown)+LocalServer".to_string(),
            );
        }
        Some("new_player_basics") => {
            return (ResolvedPhase::Sandbox, "Map(new_player_basics)".to_string());
        }
        _ => {}
    }

    if console.phase == ConsolePhase::Hideout {
        return (ResolvedPhase::Hideout, console_evidence());
    }

    if !console.matchmaking_observed
        && console.current_map.is_none()
        && state.activity == DeadlockActivity::Matchmaking
    {
        return (ResolvedPhase::Matchmaking, "MemoryMatchmaking".to_string());
    }

    if console.current_map.is_none()
        && console.server_kind != ServerKind::Local
        && state.activity == DeadlockActivity::InMatch
    {
        (ResolvedPhase::InMatch, "MemoryInMatch".to_string())
    } else {
        (ResolvedPhase::MainMenu, "MainMenuFallback".to_string())
    }
}

pub(crate) const fn match_mode_label(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::Invalid | MatchMode::Unknown(_) => "Unknown Mode",
        MatchMode::Unranked => "Unranked",
        MatchMode::PrivateLobby => "Private Lobby",
        MatchMode::CoopBot => "Co-op vs Bots",
        MatchMode::Ranked => "Ranked",
        MatchMode::ServerTest => "Server Test",
        MatchMode::Tutorial => "Tutorial",
        MatchMode::HeroLabs => "Hero Labs",
        MatchMode::NewPlayerPlacement => "Placement",
    }
}

pub(crate) fn normalize_party_size(size: u32, max: u32) -> u32 {
    if max > 0 && (1..=max).contains(&size) {
        size
    } else {
        1
    }
}

pub(crate) fn presence_changed(
    published: Option<&PresenceSnapshot>,
    desired: Option<&PresenceSnapshot>,
) -> bool {
    match (published, desired) {
        (None, None) => false,
        (Some(current), Some(next)) => {
            current.activity != next.activity
                || current.match_mode != next.match_mode
                || current.game_mode != next.game_mode
                || current.resolved_phase != next.resolved_phase
                || current.details != next.details
                || current.state != next.state
                || current.party_size != next.party_size
                || current.party_max != next.party_max
                || current.started_at != next.started_at
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deadlock_state(
        activity: DeadlockActivity,
        match_mode: MatchMode,
        game_mode: GameMode,
    ) -> DeadlockState {
        DeadlockState {
            activity,
            match_mode,
            game_mode,
            party_size: 2,
            party_max: 6,
        }
    }

    fn console(phase: ConsolePhase, map: Option<&str>, evidence: &str) -> ConsolePhaseState {
        console_with_server(phase, map, ServerKind::Unknown, evidence)
    }

    fn console_with_server(
        phase: ConsolePhase,
        map: Option<&str>,
        server_kind: ServerKind,
        evidence: &str,
    ) -> ConsolePhaseState {
        ConsolePhaseState {
            phase,
            current_map: map.map(str::to_string),
            server_kind,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            evidence: Some(evidence.to_string()),
            last_update_ms: 1,
        }
    }

    fn snapshot(memory: DeadlockState, console: &ConsolePhaseState) -> PresenceSnapshot {
        PresenceModel::default()
            .observe_with_console(true, Some(memory), console, 100)
            .unwrap()
    }

    fn active_matchmaking_console(map: &str, server_kind: ServerKind) -> ConsolePhaseState {
        let mut state = console_with_server(
            if map == "dl_hideout" {
                ConsolePhase::Hideout
            } else {
                ConsolePhase::Unknown
            },
            Some(map),
            server_kind,
            "GCStartMatchmaking",
        );
        state.matchmaking_active = true;
        state.matchmaking_observed = true;
        state
    }

    #[test]
    fn match_modes_have_clean_labels() {
        assert_eq!(match_mode_label(MatchMode::Unranked), "Unranked");
        assert_eq!(match_mode_label(MatchMode::Ranked), "Ranked");
        assert_eq!(match_mode_label(MatchMode::PrivateLobby), "Private Lobby");
        assert_eq!(match_mode_label(MatchMode::CoopBot), "Co-op vs Bots");
        assert_eq!(match_mode_label(MatchMode::HeroLabs), "Hero Labs");
        assert_eq!(match_mode_label(MatchMode::NewPlayerPlacement), "Placement");
        assert_eq!(match_mode_label(MatchMode::Unknown(17)), "Unknown Mode");
    }

    #[test]
    fn local_hideout_map_resolves_to_hideout() {
        let result = snapshot(
            deadlock_state(
                DeadlockActivity::Idle,
                MatchMode::Invalid,
                GameMode::Invalid,
            ),
            &console_with_server(
                ConsolePhase::Hideout,
                Some("dl_hideout"),
                ServerKind::Local,
                "Map(dl_hideout)",
            ),
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::Hideout);
    }

    #[test]
    fn local_midtown_resolves_to_explore_nyc_without_memory_modes() {
        let result = snapshot(
            deadlock_state(
                DeadlockActivity::Idle,
                MatchMode::Invalid,
                GameMode::Invalid,
            ),
            &console_with_server(
                ConsolePhase::Unknown,
                Some("dl_midtown"),
                ServerKind::Local,
                "ChangeGameState(7)",
            ),
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::ExploreNyc);
        assert_eq!(result.evidence, "Map(dl_midtown)+LocalServer");
    }

    #[test]
    fn console_matchmaking_has_priority_over_hideout_and_local_midtown() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );

        for console in [
            active_matchmaking_console("dl_hideout", ServerKind::Local),
            active_matchmaking_console("dl_midtown", ServerKind::Local),
        ] {
            let result = snapshot(memory, &console);
            assert_eq!(result.resolved_phase, ResolvedPhase::Matchmaking);
            assert_eq!(result.evidence, "GCStartMatchmaking");
            assert_eq!(
                (result.details, result.state),
                ("Searching for Match", "Matchmaking")
            );
        }
    }

    #[test]
    fn stopped_console_matchmaking_returns_to_map_phase() {
        let memory = deadlock_state(
            DeadlockActivity::Matchmaking,
            MatchMode::Ranked,
            GameMode::Normal,
        );

        let mut hideout = console_with_server(
            ConsolePhase::Hideout,
            Some("dl_hideout"),
            ServerKind::Local,
            "GCStopMatchmaking",
        );
        hideout.matchmaking_observed = true;
        assert_eq!(
            snapshot(memory, &hideout).resolved_phase,
            ResolvedPhase::Hideout
        );

        let mut explore = console_with_server(
            ConsolePhase::InMatch,
            Some("dl_midtown"),
            ServerKind::Local,
            "GCStopMatchmaking",
        );
        explore.matchmaking_observed = true;
        assert_eq!(
            snapshot(memory, &explore).resolved_phase,
            ResolvedPhase::ExploreNyc
        );
    }

    #[test]
    fn remote_midtown_unranked_and_ranked_remain_in_match() {
        for mode in [MatchMode::Unranked, MatchMode::Ranked] {
            let result = snapshot(
                deadlock_state(DeadlockActivity::InMatch, mode, GameMode::Normal),
                &console_with_server(
                    ConsolePhase::InMatch,
                    Some("dl_midtown"),
                    ServerKind::Remote,
                    "ChangeGameState(7)",
                ),
            );
            assert_eq!(result.resolved_phase, ResolvedPhase::InMatch);
            assert_eq!(result.state, "Deadlock");
        }
    }

    #[test]
    fn new_player_basics_resolves_to_sandbox() {
        let result = snapshot(
            deadlock_state(
                DeadlockActivity::InMatch,
                MatchMode::Unranked,
                GameMode::Sandbox,
            ),
            &console(
                ConsolePhase::InMatch,
                Some("new_player_basics"),
                "Map(new_player_basics)",
            ),
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::Sandbox);
        assert_eq!((result.details, result.state), ("Practice", "Sandbox"));
    }

    #[test]
    fn explicit_spectating_has_priority_over_memory_matchmaking() {
        let result = snapshot(
            deadlock_state(
                DeadlockActivity::Matchmaking,
                MatchMode::Ranked,
                GameMode::Normal,
            ),
            &console(
                ConsolePhase::Spectating,
                Some("dl_midtown"),
                "PlayingBroadcast",
            ),
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::Spectating);
        assert_eq!(result.state, "Deadlock");
        assert_eq!((result.party_size, result.party_max), (2, 6));
        assert_eq!(result.started_at, None);
    }

    #[test]
    fn console_match_found_resolves_loading_without_memory() {
        let mut console = ConsolePhaseState::default();
        console.phase = ConsolePhase::Loading;
        console.match_found = true;
        console.matchmaking_active = true;
        console.matchmaking_observed = true;
        console.evidence = Some("GCMatchFound".to_string());

        let result = PresenceModel::default()
            .observe_with_console(true, None, &console, 100)
            .unwrap();

        assert_eq!(result.resolved_phase, ResolvedPhase::Loading);
        assert_eq!(
            (result.details, result.state),
            ("Loading into Match", "Deadlock")
        );
    }

    #[test]
    fn explicit_console_phases_are_exposed() {
        let memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Unranked,
            GameMode::Normal,
        );
        for (phase, resolved) in [
            (ConsolePhase::Loading, ResolvedPhase::Loading),
            (ConsolePhase::InMatch, ResolvedPhase::InMatch),
            (ConsolePhase::PostMatch, ResolvedPhase::PostMatch),
            (ConsolePhase::Spectating, ResolvedPhase::Spectating),
        ] {
            assert_eq!(
                snapshot(memory, &console(phase, None, "test")).resolved_phase,
                resolved
            );
        }
    }

    #[test]
    fn idle_without_known_map_or_active_console_phase_falls_back_to_main_menu() {
        let result = snapshot(
            deadlock_state(
                DeadlockActivity::Idle,
                MatchMode::Invalid,
                GameMode::Invalid,
            ),
            &ConsolePhaseState::default(),
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(result.started_at, None);
    }

    #[test]
    fn party_size_falls_back_to_one() {
        assert_eq!(normalize_party_size(0, 6), 1);
        assert_eq!(normalize_party_size(7, 6), 1);
        assert_eq!(normalize_party_size(3, 6), 3);
    }

    #[test]
    fn identical_presence_is_deduplicated() {
        let memory = deadlock_state(
            DeadlockActivity::Matchmaking,
            MatchMode::Ranked,
            GameMode::Normal,
        );
        let base_console = ConsolePhaseState::default();
        let mut model = PresenceModel::default();
        let first = model.observe_with_console(true, Some(memory), &base_console, 100);
        let second = model.observe_with_console(true, Some(memory), &base_console, 200);
        assert!(!presence_changed(first.as_ref(), second.as_ref()));
    }

    #[test]
    fn evidence_only_changes_do_not_republish_presence() {
        let memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Unranked,
            GameMode::Normal,
        );
        let first = snapshot(
            memory,
            &console(ConsolePhase::InMatch, None, "ChangeGameState(7)"),
        );
        let second = snapshot(
            memory,
            &console(ConsolePhase::InMatch, None, "Map(dl_midtown)"),
        );
        assert!(!presence_changed(Some(&first), Some(&second)));
    }

    #[test]
    fn matchmaking_timestamp_survives_mode_changes() {
        let base_console = ConsolePhaseState::default();
        let mut model = PresenceModel::default();
        let first = model
            .observe_with_console(
                true,
                Some(deadlock_state(
                    DeadlockActivity::Matchmaking,
                    MatchMode::Unranked,
                    GameMode::Normal,
                )),
                &base_console,
                100,
            )
            .unwrap();
        let second = model
            .observe_with_console(
                true,
                Some(deadlock_state(
                    DeadlockActivity::Matchmaking,
                    MatchMode::Ranked,
                    GameMode::Normal,
                )),
                &base_console,
                500,
            )
            .unwrap();
        assert_eq!(first.started_at, Some(100));
        assert_eq!(second.started_at, Some(100));
    }

    #[test]
    fn entering_match_starts_a_new_timestamp_and_keeps_it_stable() {
        let mut model = PresenceModel::default();
        let base_console = ConsolePhaseState::default();
        model.observe_with_console(
            true,
            Some(deadlock_state(
                DeadlockActivity::Matchmaking,
                MatchMode::Ranked,
                GameMode::Normal,
            )),
            &base_console,
            100,
        );

        let in_match_console = console(ConsolePhase::InMatch, None, "ChangeGameState(7)");
        let memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Ranked,
            GameMode::Normal,
        );
        let first = model
            .observe_with_console(true, Some(memory), &in_match_console, 300)
            .unwrap();
        let second = model
            .observe_with_console(true, Some(memory), &in_match_console, 900)
            .unwrap();

        assert_eq!(first.started_at, Some(300));
        assert_eq!(second.started_at, Some(300));
    }

    #[test]
    fn returning_to_main_menu_resets_timestamps() {
        let mut model = PresenceModel::default();
        let in_match_console = console(ConsolePhase::InMatch, None, "ChangeGameState(7)");
        model.observe_with_console(
            true,
            Some(deadlock_state(
                DeadlockActivity::InMatch,
                MatchMode::Ranked,
                GameMode::Normal,
            )),
            &in_match_console,
            100,
        );

        let menu = model
            .observe_with_console(
                true,
                Some(deadlock_state(
                    DeadlockActivity::Idle,
                    MatchMode::Invalid,
                    GameMode::Invalid,
                )),
                &ConsolePhaseState::default(),
                200,
            )
            .unwrap();
        assert_eq!(menu.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(menu.started_at, None);
    }

    #[test]
    fn disabled_presence_returns_none_and_resets_timer() {
        let memory = deadlock_state(
            DeadlockActivity::Matchmaking,
            MatchMode::Ranked,
            GameMode::Normal,
        );
        let console = ConsolePhaseState::default();
        let mut model = PresenceModel::default();
        model.observe_with_console(true, Some(memory), &console, 100);
        assert_eq!(model.observe_with_console(false, None, &console, 200), None);
        let next = model
            .observe_with_console(true, Some(memory), &console, 300)
            .unwrap();
        assert_eq!(next.started_at, Some(300));
    }
}
