use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
use std::cell::Cell;

use tauri::AppHandle;

use super::{
    console_phase::{ConsolePhase, ConsolePhaseState, ServerKind},
    deadlock_state::{DeadlockActivity, DeadlockState, GameMode, MatchMode},
};

pub(crate) const MATCH_SAFETY_ERROR: &str = "SPLIT save states are disabled during a live match.";

static MATCH_SAFETY_LOCKED: AtomicBool = AtomicBool::new(false);
static POST_MATCH_LOGGED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    static TEST_LOCK_OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafetyDecision {
    Lock(&'static str),
    Unlock(&'static str),
    Retain,
}

pub(crate) fn is_locked() -> bool {
    #[cfg(test)]
    if let Some(locked) = TEST_LOCK_OVERRIDE.with(Cell::get) {
        return locked;
    }

    MATCH_SAFETY_LOCKED.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn with_test_lock<T>(locked: bool, operation: impl FnOnce() -> T) -> T {
    TEST_LOCK_OVERRIDE.with(|override_value| {
        let previous = override_value.replace(Some(locked));
        let result = operation();
        override_value.set(previous);
        result
    })
}

pub(crate) fn ensure_actions_allowed() -> Result<(), String> {
    ensure_actions_allowed_when(is_locked())
}

fn ensure_actions_allowed_when(locked: bool) -> Result<(), String> {
    if locked {
        Err(MATCH_SAFETY_ERROR.to_string())
    } else {
        Ok(())
    }
}

fn is_live_match_mode(mode: MatchMode) -> bool {
    matches!(
        mode,
        MatchMode::Unranked
            | MatchMode::PrivateLobby
            | MatchMode::CoopBot
            | MatchMode::Ranked
            | MatchMode::HeroLabs
            | MatchMode::NewPlayerPlacement
    )
}

fn decide(
    currently_locked: bool,
    memory: Option<DeadlockState>,
    console: &ConsolePhaseState,
    hideout_ready: bool,
) -> SafetyDecision {
    if currently_locked {
        return if hideout_ready && console.current_map.as_deref() == Some("dl_hideout") {
            SafetyDecision::Unlock("hideout confirmed")
        } else {
            SafetyDecision::Retain
        };
    }

    if console.broadcast_active {
        return SafetyDecision::Unlock("spectating");
    }

    if console.matchmaking_active {
        return SafetyDecision::Unlock("matchmaking");
    }

    if console.server_kind == ServerKind::Local {
        return SafetyDecision::Unlock("local practice context");
    }

    if matches!(
        console.current_map.as_deref(),
        Some("dl_hideout") | Some("new_player_basics")
    ) {
        return SafetyDecision::Unlock("safe map");
    }

    if matches!(
        console.phase,
        ConsolePhase::Hideout | ConsolePhase::Spectating
    ) {
        return SafetyDecision::Unlock("safe console phase");
    }

    if console.phase == ConsolePhase::InMatch {
        return SafetyDecision::Lock("console match in progress");
    }

    if let Some(memory) = memory {
        if matches!(memory.game_mode, GameMode::Sandbox | GameMode::OneVsOneTest)
            || matches!(
                memory.match_mode,
                MatchMode::ServerTest | MatchMode::Tutorial
            )
        {
            return SafetyDecision::Unlock("practice game mode");
        }

        if memory.activity == DeadlockActivity::Matchmaking {
            return SafetyDecision::Unlock("memory matchmaking");
        }

        if memory.activity == DeadlockActivity::InMatch
            && memory.game_mode == GameMode::Normal
            && is_live_match_mode(memory.match_mode)
        {
            return SafetyDecision::Lock("memory live match");
        }
    }

    if console.phase == ConsolePhase::MainMenu {
        return SafetyDecision::Unlock("confirmed main menu");
    }

    SafetyDecision::Retain
}

fn next_locked(current: bool, running: bool, decision: SafetyDecision) -> bool {
    if !running {
        return false;
    }

    match decision {
        SafetyDecision::Lock(_) => true,
        SafetyDecision::Unlock(_) => false,
        SafetyDecision::Retain => current,
    }
}

pub(crate) fn refresh(app: &AppHandle, running: bool) {
    let memory = running
        .then(super::deadlock_state::read_deadlock_state)
        .and_then(Result::ok)
        .flatten();
    let console = super::console_phase::snapshot();
    let previous = is_locked();
    let decision = decide(
        previous,
        memory,
        &console,
        super::console_phase::hideout_ready(),
    );
    let next = next_locked(previous, running, decision);

    if previous
        && console.phase == ConsolePhase::PostMatch
        && !POST_MATCH_LOGGED.swap(true, Ordering::SeqCst)
    {
        println!("[SPLIT][Safety] post-match detected; keeping lock");
    }

    if previous == next {
        return;
    }

    MATCH_SAFETY_LOCKED.store(next, Ordering::SeqCst);
    let reason = match decision {
        SafetyDecision::Lock(reason) | SafetyDecision::Unlock(reason) => reason,
        SafetyDecision::Retain => "Deadlock stopped",
    };
    if next {
        POST_MATCH_LOGGED.store(false, Ordering::SeqCst);
        println!("[SPLIT][Safety] live match lock enabled ({reason})");
    } else if reason == "hideout confirmed" {
        POST_MATCH_LOGGED.store(false, Ordering::SeqCst);
        println!("[SPLIT][Safety] hideout confirmed; live match lock disabled");
    } else {
        POST_MATCH_LOGGED.store(false, Ordering::SeqCst);
        println!("[SPLIT][Safety] live match lock disabled ({reason})");
    }

    if next {
        super::watcher::cancel_pending_save_for_match_lock();
    }
    if next && crate::editions::windows_quick_access_enabled() {
        if let Err(error) = crate::quick_access::hide_without_focus(app) {
            eprintln!("[SPLIT][Safety] Could not close Quick Access: {error}");
        }
    }
    if let Err(error) = super::hotkeys::refresh_quick_access_hotkeys() {
        eprintln!("[SPLIT][Safety] Could not refresh Quick Access hotkey: {error}");
    }

    crate::ui::emit_to_main_if_present(app, "deadlock-match-safety-changed", next);
    crate::ui::emit_to_main_if_present(app, "deadlock-status-changed", super::get_status());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(
        activity: DeadlockActivity,
        match_mode: MatchMode,
        game_mode: GameMode,
    ) -> DeadlockState {
        DeadlockState {
            activity,
            match_mode,
            game_mode,
            party_size: 1,
            party_max: 6,
        }
    }

    fn console(phase: ConsolePhase, map: Option<&str>, server: ServerKind) -> ConsolePhaseState {
        ConsolePhaseState {
            phase,
            current_map: map.map(str::to_string),
            server_kind: server,
            broadcast_active: false,
            spectating_match_id: None,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            current_hero: None,
            hero_capture_mode: super::super::console_phase::HeroCaptureMode::Disabled,
            evidence: Some("test".to_string()),
            last_update_ms: 1,
        }
    }

    #[test]
    fn safe_contexts_are_unlocked_before_a_match() {
        for state in [
            console(ConsolePhase::MainMenu, None, ServerKind::Unknown),
            console(ConsolePhase::Hideout, Some("dl_hideout"), ServerKind::Local),
            console(ConsolePhase::Unknown, Some("dl_midtown"), ServerKind::Local),
            console(
                ConsolePhase::Unknown,
                Some("new_player_basics"),
                ServerKind::Local,
            ),
        ] {
            assert!(matches!(
                decide(false, None, &state, false),
                SafetyDecision::Unlock(_)
            ));
        }

        let mut matchmaking = console(ConsolePhase::Loading, None, ServerKind::Unknown);

        matchmaking.matchmaking_active = true;

        assert!(matches!(
            decide(false, None, &matchmaking, false),
            SafetyDecision::Unlock(_)
        ));
    }

    #[test]
    fn sandbox_and_practice_memory_are_unlocked() {
        let unknown = console(ConsolePhase::Unknown, None, ServerKind::Unknown);

        for (match_mode, game_mode) in [
            (MatchMode::Unranked, GameMode::Sandbox),
            (MatchMode::Unranked, GameMode::OneVsOneTest),
            (MatchMode::Tutorial, GameMode::Normal),
            (MatchMode::ServerTest, GameMode::Normal),
        ] {
            assert!(matches!(
                decide(
                    false,
                    Some(memory(DeadlockActivity::InMatch, match_mode, game_mode)),
                    &unknown,
                    false,
                ),
                SafetyDecision::Unlock(_)
            ));
        }
    }

    #[test]
    fn real_remote_or_memory_match_locks() {
        let remote = console(
            ConsolePhase::InMatch,
            Some("dl_midtown"),
            ServerKind::Remote,
        );

        assert!(matches!(
            decide(false, None, &remote, false),
            SafetyDecision::Lock(_)
        ));

        let replay_without_endpoint = console(
            ConsolePhase::InMatch,
            Some("dl_midtown"),
            ServerKind::Unknown,
        );

        assert!(matches!(
            decide(false, None, &replay_without_endpoint, false),
            SafetyDecision::Lock(_)
        ));

        let unknown = console(ConsolePhase::Unknown, None, ServerKind::Unknown);

        assert!(matches!(
            decide(
                false,
                Some(memory(
                    DeadlockActivity::InMatch,
                    MatchMode::Ranked,
                    GameMode::Normal
                )),
                &unknown,
                false,
            ),
            SafetyDecision::Lock(_)
        ));
    }

    #[test]
    fn post_match_keeps_existing_lock() {
        let post_match = console(
            ConsolePhase::PostMatch,
            Some("dl_midtown"),
            ServerKind::Remote,
        );

        assert_eq!(
            decide(true, None, &post_match, false),
            SafetyDecision::Retain
        );
    }

    #[test]
    fn loading_unknown_and_transient_states_keep_existing_lock() {
        for state in [
            console(ConsolePhase::Loading, None, ServerKind::Remote),
            console(ConsolePhase::Unknown, None, ServerKind::Unknown),
            console(ConsolePhase::MainMenu, None, ServerKind::Unknown),
        ] {
            assert_eq!(decide(true, None, &state, false), SafetyDecision::Retain);
        }
    }

    #[test]
    fn hideout_map_alone_does_not_unlock_latched_match() {
        let hideout = console(ConsolePhase::Hideout, Some("dl_hideout"), ServerKind::Local);

        assert_eq!(decide(true, None, &hideout, false), SafetyDecision::Retain);
    }

    #[test]
    fn confirmed_hideout_ready_unlocks_latched_match() {
        let hideout = console(ConsolePhase::Hideout, Some("dl_hideout"), ServerKind::Local);

        assert_eq!(
            decide(true, None, &hideout, true),
            SafetyDecision::Unlock("hideout confirmed")
        );
    }

    #[test]
    fn matchmaking_after_latched_match_does_not_unlock() {
        let mut matchmaking = console(ConsolePhase::Loading, None, ServerKind::Unknown);

        matchmaking.matchmaking_active = true;

        assert_eq!(
            decide(true, None, &matchmaking, false),
            SafetyDecision::Retain
        );
    }

    #[test]
    fn process_stop_always_clears_latched_lock() {
        assert!(!next_locked(true, false, SafetyDecision::Retain));
    }

    #[test]
    fn transient_failure_does_not_unlock() {
        let loading = console(ConsolePhase::Loading, None, ServerKind::Remote);

        assert_eq!(decide(true, None, &loading, false), SafetyDecision::Retain);

        assert!(next_locked(true, true, SafetyDecision::Retain));
    }

    #[test]
    fn backend_guard_refuses_without_running_mutation() {
        let mut data = vec!["slot one".to_string()];
        let before = data.clone();

        assert_eq!(
            ensure_actions_allowed_when(true),
            Err(MATCH_SAFETY_ERROR.to_string())
        );

        assert_eq!(data, before);

        assert!(ensure_actions_allowed_when(false).is_ok());

        data.push("slot two".to_string());

        assert_eq!(data.len(), 2);
    }
}
