#[cfg(debug_assertions)]
use std::sync::Mutex;

use serde::Serialize;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
};

use super::{camera, process};

const CLIENT_MODULE: &str = "client.dll";
const PARTY_MAX: u32 = 6;

/*
 * Deadlock offsets for the client.dll build identified by:
 *
 * SHA256: 07F65AB6F862517EF6B1049679F78342D572ACC589CBA54E9851D61380B19E6E
 * Date:   2026-10-01 01:58:46
 *
 * Keep every version-dependent address here so a future Deadlock update
 * can be supported without changing the state-reading logic below.
 */
#[derive(Debug, Clone, Copy)]
struct DeadlockOffsets {
    queue_state: usize,
    queue_match_mode: usize,
    queue_game_mode: usize,

    game_rules: usize,
    game_rules_vtable: usize,
    game_rules_match_mode: usize,
    game_rules_game_mode: usize,

    gc_system: usize,
    gc_system_ptr: usize,
    gc_system_vtable: usize,
    gc_party_ptr: usize,
    party_size: usize,
}

const OFFSETS: DeadlockOffsets = DeadlockOffsets {
    queue_state: 0x3BE5144,
    queue_match_mode: 0x324B378,
    queue_game_mode: 0x324B37C,

    game_rules: 0x3BC50D8,
    game_rules_vtable: 0x265C108,
    game_rules_match_mode: 0xA8,
    game_rules_game_mode: 0xAC,

    gc_system: 0x37DF730,
    gc_system_ptr: 0x37DFB10,
    gc_system_vtable: 0x2720608,
    gc_party_ptr: 0x630,
    party_size: 0x28,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeadlockActivity {
    Idle,
    Matchmaking,
    InMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchMode {
    Invalid,
    Unranked,
    PrivateLobby,
    CoopBot,
    Ranked,
    ServerTest,
    Tutorial,
    HeroLabs,
    NewPlayerPlacement,
    Unknown(i32),
}

impl From<i32> for MatchMode {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Invalid,
            1 => Self::Unranked,
            2 => Self::PrivateLobby,
            3 => Self::CoopBot,
            4 => Self::Ranked,
            5 => Self::ServerTest,
            6 => Self::Tutorial,
            7 => Self::HeroLabs,
            8 => Self::NewPlayerPlacement,
            value => Self::Unknown(value),
        }
    }
}

const fn is_recognized_match_mode(mode: MatchMode) -> bool {
    matches!(
        mode,
        MatchMode::Unranked
            | MatchMode::PrivateLobby
            | MatchMode::CoopBot
            | MatchMode::Ranked
            | MatchMode::ServerTest
            | MatchMode::Tutorial
            | MatchMode::HeroLabs
            | MatchMode::NewPlayerPlacement
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GameMode {
    Invalid,
    Normal,
    OneVsOneTest,
    Sandbox,
    Unknown(i32),
}

impl From<i32> for GameMode {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Invalid,
            1 => Self::Normal,
            2 => Self::OneVsOneTest,
            3 => Self::Sandbox,
            value => Self::Unknown(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlockState {
    pub activity: DeadlockActivity,
    pub match_mode: MatchMode,
    pub game_mode: GameMode,
    pub party_size: u32,
    pub party_max: u32,
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open_read_only(pid: u32) -> Result<Self, String> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };

        if handle.is_null() {
            return Err(format!(
                "Could not open Deadlock for read-only state inspection: {}",
                std::io::Error::last_os_error(),
            ));
        }

        Ok(Self(handle))
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RawDeadlockState {
    queue_flag: bool,
    queue_match_mode: Option<i32>,
    queue_game_mode: Option<i32>,
    game_rules_modes: Option<(i32, i32)>,
    party_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameRulesReadReason {
    Accepted,
    GlobalAddressOverflow,
    GlobalReadFailed,
    NullPointer,
    InvalidPointer,
    VtableReadFailed,
    ExpectedVtableAddressOverflow,
    VtableMismatch,
    MatchModeReadFailed,
    GameModeReadFailed,
}

impl GameRulesReadReason {
    const fn label(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::GlobalAddressOverflow => "rules_global_address_overflow",
            Self::GlobalReadFailed => "rules_global_read_failed",
            Self::NullPointer => "rules_ptr_null",
            Self::InvalidPointer => "rules_ptr_invalid",
            Self::VtableReadFailed => "rules_vtable_read_failed",
            Self::ExpectedVtableAddressOverflow => "expected_vtable_address_overflow",
            Self::VtableMismatch => "rules_vtable_mismatch",
            Self::MatchModeReadFailed => "raw_match_mode_read_failed",
            Self::GameModeReadFailed => "raw_game_mode_read_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GameRulesRead {
    client_base: usize,
    rules_global: Option<usize>,
    rules_ptr: Option<usize>,
    rules_vtable: Option<usize>,
    expected_vtable: Option<usize>,
    raw_match_mode: Option<i32>,
    raw_game_mode: Option<i32>,
    accepted: bool,
    reason: GameRulesReadReason,
}

impl GameRulesRead {
    const fn initial(client_base: usize) -> Self {
        Self {
            client_base,
            rules_global: None,
            rules_ptr: None,
            rules_vtable: None,
            expected_vtable: None,
            raw_match_mode: None,
            raw_game_mode: None,
            accepted: false,
            reason: GameRulesReadReason::GlobalAddressOverflow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchmakingRead {
    byte0: Option<u8>,
    byte1: Option<u8>,
    raw_match_mode: Option<i32>,
    raw_game_mode: Option<i32>,
    queue_flag: bool,
}

fn checked_address(base: usize, offset: usize) -> Option<usize> {
    base.checked_add(offset)
}

fn read_at<T: Copy>(process: HANDLE, base: usize, offset: usize) -> Option<T> {
    let address = checked_address(base, offset)?;
    camera::read_value(process, address).ok()
}

fn valid_pointer(pointer: usize) -> bool {
    pointer >= 0x10000
}

fn read_matchmaking(process: HANDLE, client_base: usize) -> MatchmakingRead {
    let byte0 = read_at::<u8>(process, client_base, OFFSETS.queue_state);
    let byte1 = read_at::<u8>(process, client_base, OFFSETS.queue_state + 1);
    let queue_flag = matches!((byte0, byte1), (Some(a), Some(b)) if a != 0 && b != 0);

    MatchmakingRead {
        byte0,
        byte1,
        raw_match_mode: read_at(process, client_base, OFFSETS.queue_match_mode),
        raw_game_mode: read_at(process, client_base, OFFSETS.queue_game_mode),
        queue_flag,
    }
}

fn has_valid_matchmaking_queue(raw: RawDeadlockState) -> bool {
    raw.queue_flag
        && raw
            .queue_match_mode
            .map(MatchMode::from)
            .is_some_and(is_recognized_match_mode)
}

fn read_game_rules_modes(
    process: HANDLE,
    client_base: usize,
) -> (Option<(i32, i32)>, GameRulesRead) {
    let mut diagnostic = GameRulesRead::initial(client_base);

    let Some(rules_global) = checked_address(client_base, OFFSETS.game_rules) else {
        return (None, diagnostic);
    };
    diagnostic.rules_global = Some(rules_global);
    diagnostic.expected_vtable = checked_address(client_base, OFFSETS.game_rules_vtable);

    let Ok(rules) = camera::read_value::<usize>(process, rules_global) else {
        diagnostic.reason = GameRulesReadReason::GlobalReadFailed;
        return (None, diagnostic);
    };
    diagnostic.rules_ptr = Some(rules);

    if !valid_pointer(rules) {
        diagnostic.reason = if rules == 0 {
            GameRulesReadReason::NullPointer
        } else {
            GameRulesReadReason::InvalidPointer
        };
        return (None, diagnostic);
    }

    let Ok(actual_vtable) = camera::read_value::<usize>(process, rules) else {
        diagnostic.reason = GameRulesReadReason::VtableReadFailed;
        return (None, diagnostic);
    };
    diagnostic.rules_vtable = Some(actual_vtable);

    let Some(expected_vtable) = diagnostic.expected_vtable else {
        diagnostic.reason = GameRulesReadReason::ExpectedVtableAddressOverflow;
        return (None, diagnostic);
    };

    if actual_vtable != expected_vtable {
        diagnostic.reason = GameRulesReadReason::VtableMismatch;
        return (None, diagnostic);
    }

    let Some(match_mode) = read_at::<i32>(process, rules, OFFSETS.game_rules_match_mode) else {
        diagnostic.reason = GameRulesReadReason::MatchModeReadFailed;
        return (None, diagnostic);
    };
    diagnostic.raw_match_mode = Some(match_mode);

    let Some(game_mode) = read_at::<i32>(process, rules, OFFSETS.game_rules_game_mode) else {
        diagnostic.reason = GameRulesReadReason::GameModeReadFailed;
        return (None, diagnostic);
    };
    diagnostic.raw_game_mode = Some(game_mode);
    diagnostic.accepted = true;
    diagnostic.reason = GameRulesReadReason::Accepted;

    (Some((match_mode, game_mode)), diagnostic)
}

fn normalize_party_size(value: Option<u32>) -> u32 {
    match value {
        Some(value @ 1..=PARTY_MAX) => value,
        _ => 1,
    }
}

fn read_party_size(process: HANDLE, client_base: usize) -> u32 {
    let Some(manager) = checked_address(client_base, OFFSETS.gc_system) else {
        return 1;
    };

    let manager_pointer = read_at::<usize>(process, client_base, OFFSETS.gc_system_ptr);
    if manager_pointer != Some(manager) {
        return 1;
    }

    let expected_vtable = checked_address(client_base, OFFSETS.gc_system_vtable);
    let actual_vtable = camera::read_value::<usize>(process, manager).ok();
    if actual_vtable != expected_vtable {
        return 1;
    }

    let party = read_at::<usize>(process, manager, OFFSETS.gc_party_ptr);
    let Some(party) = party.filter(|pointer| valid_pointer(*pointer)) else {
        return 1;
    };

    /*
     * Best effort only: party + 0x28 is inferred from the game's
     * NewMember notification path and has not yet been runtime-validated
     * with another party member. Invalid values deliberately fall back to 1.
     */
    normalize_party_size(read_at::<u32>(process, party, OFFSETS.party_size))
}

fn state_from_raw(raw: RawDeadlockState) -> DeadlockState {
    let (activity, match_mode, game_mode) = if has_valid_matchmaking_queue(raw) {
        (
            DeadlockActivity::Matchmaking,
            raw.queue_match_mode
                .map(MatchMode::from)
                .unwrap_or(MatchMode::Invalid),
            raw.queue_game_mode
                .map(GameMode::from)
                .unwrap_or(GameMode::Invalid),
        )
    } else if let Some((raw_match_mode, raw_game_mode)) = raw.game_rules_modes {
        let match_mode = MatchMode::from(raw_match_mode);
        let game_mode = GameMode::from(raw_game_mode);
        let activity = if is_recognized_match_mode(match_mode) {
            DeadlockActivity::InMatch
        } else {
            DeadlockActivity::Idle
        };

        (activity, match_mode, game_mode)
    } else {
        (
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        )
    };

    DeadlockState {
        activity,
        match_mode,
        game_mode,
        party_size: normalize_party_size(Some(raw.party_size)),
        party_max: PARTY_MAX,
    }
}

fn game_rules_rejected_as_non_match(raw: RawDeadlockState) -> bool {
    !has_valid_matchmaking_queue(raw)
        && raw
            .game_rules_modes
            .is_some_and(|(match_mode, _)| !is_recognized_match_mode(MatchMode::from(match_mode)))
}

#[cfg(debug_assertions)]
fn log_state_change(state: DeadlockState, rejected_game_rules: bool) {
    static LAST_LOGGED_STATE: Mutex<Option<(DeadlockState, bool)>> = Mutex::new(None);

    let Ok(mut last) = LAST_LOGGED_STATE.lock() else {
        return;
    };

    if last.as_ref() == Some(&(state, rejected_game_rules)) {
        return;
    }

    if rejected_game_rules {
        println!(
            "[SPLIT][Debug] DeadlockState: GameRules present but non-match mode: match_mode={:?} game_mode={:?}",
            state.match_mode, state.game_mode,
        );
    } else {
        println!(
            "[SPLIT][Debug] DeadlockState: activity={:?} match_mode={:?} game_mode={:?} party={}/{}",
            state.activity, state.match_mode, state.game_mode, state.party_size, state.party_max,
        );
    }

    *last = Some((state, rejected_game_rules));
}

#[cfg(not(debug_assertions))]
fn log_state_change(_state: DeadlockState, _rejected_game_rules: bool) {}



pub fn read_deadlock_state() -> Result<Option<DeadlockState>, String> {
    let Some(pid) = process::deadlock_pid() else {
        return Ok(None);
    };

    let process = ProcessHandle::open_read_only(pid)?;
    let (client_base, _) = camera::find_module(pid, CLIENT_MODULE)?;

    let queue_read = read_matchmaking(process.0, client_base);
    let queue_flag = queue_read.queue_flag;
    let queue_match_mode = if queue_flag {
        queue_read.raw_match_mode
    } else {
        None
    };
    let queue_game_mode = if queue_flag {
        queue_read.raw_game_mode
    } else {
        None
    };

    let queue_state = RawDeadlockState {
        queue_flag,
        queue_match_mode,
        queue_game_mode,
        ..RawDeadlockState::default()
    };
    let (observed_game_rules_modes, _game_rules_diagnostic) =
        read_game_rules_modes(process.0, client_base);
    let game_rules_modes = if has_valid_matchmaking_queue(queue_state) {
        None
    } else {
        observed_game_rules_modes
    };

    let raw = RawDeadlockState {
        queue_flag,
        queue_match_mode,
        queue_game_mode,
        game_rules_modes,
        party_size: read_party_size(process.0, client_base),
    };

    let state = state_from_raw(raw);
    log_state_change(state, game_rules_rejected_as_non_match(raw));
    

    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_mode_mapping_preserves_known_and_unknown_values() {
        assert_eq!(MatchMode::from(0), MatchMode::Invalid);
        assert_eq!(MatchMode::from(1), MatchMode::Unranked);
        assert_eq!(MatchMode::from(4), MatchMode::Ranked);
        assert_eq!(MatchMode::from(8), MatchMode::NewPlayerPlacement);
        assert_eq!(MatchMode::from(99), MatchMode::Unknown(99));
    }

    #[test]
    fn game_mode_mapping_preserves_known_and_unknown_values() {
        assert_eq!(GameMode::from(0), GameMode::Invalid);
        assert_eq!(GameMode::from(1), GameMode::Normal);
        assert_eq!(GameMode::from(2), GameMode::OneVsOneTest);
        assert_eq!(GameMode::from(3), GameMode::Sandbox);
        assert_eq!(GameMode::from(-1), GameMode::Unknown(-1));
    }

    #[test]
    fn party_size_uses_only_sane_values() {
        assert_eq!(normalize_party_size(None), 1);
        assert_eq!(normalize_party_size(Some(0)), 1);
        assert_eq!(normalize_party_size(Some(1)), 1);
        assert_eq!(normalize_party_size(Some(4)), 4);
        assert_eq!(normalize_party_size(Some(6)), 6);
        assert_eq!(normalize_party_size(Some(7)), 1);
    }

    fn queue_state(queue_flag: bool, raw_match_mode: i32) -> DeadlockState {
        state_from_raw(RawDeadlockState {
            queue_flag,
            queue_match_mode: Some(raw_match_mode),
            queue_game_mode: Some(0),
            party_size: 1,
            ..RawDeadlockState::default()
        })
    }

    #[test]
    fn queue_flag_with_invalid_mode_is_not_matchmaking() {
        assert_ne!(queue_state(true, 0).activity, DeadlockActivity::Matchmaking);
    }

    #[test]
    fn queue_flag_with_unknown_mode_is_not_matchmaking() {
        assert_ne!(
            queue_state(true, 99).activity,
            DeadlockActivity::Matchmaking
        );
    }

    #[test]
    fn queue_flag_with_unranked_mode_is_matchmaking() {
        let state = queue_state(true, 1);

        assert_eq!(state.activity, DeadlockActivity::Matchmaking);
        assert_eq!(state.match_mode, MatchMode::Unranked);
    }

    #[test]
    fn queue_flag_with_ranked_mode_is_matchmaking() {
        let state = queue_state(true, 4);

        assert_eq!(state.activity, DeadlockActivity::Matchmaking);
        assert_eq!(state.match_mode, MatchMode::Ranked);
    }

    #[test]
    fn ranked_mode_without_queue_flag_is_not_matchmaking() {
        assert_ne!(
            queue_state(false, 4).activity,
            DeadlockActivity::Matchmaking
        );
    }

    #[test]
    fn invalid_queue_mode_falls_through_to_valid_game_rules() {
        let state = state_from_raw(RawDeadlockState {
            queue_flag: true,
            queue_match_mode: Some(0),
            queue_game_mode: Some(0),
            game_rules_modes: Some((1, 3)),
            party_size: 1,
        });

        assert_eq!(state.activity, DeadlockActivity::InMatch);
        assert_eq!(state.match_mode, MatchMode::Unranked);
        assert_eq!(state.game_mode, GameMode::Sandbox);
    }

    #[test]
    fn matchmaking_has_priority_over_valid_game_rules() {
        let raw = RawDeadlockState {
            queue_flag: true,
            queue_match_mode: Some(4),
            queue_game_mode: Some(1),
            game_rules_modes: Some((1, 1)),
            party_size: 2,
        };
        let state = state_from_raw(raw);

        assert_eq!(state.activity, DeadlockActivity::Matchmaking);
        assert_eq!(state.match_mode, MatchMode::Ranked);
        assert_eq!(state.game_mode, GameMode::Normal);
        assert_eq!(state.party_size, 2);
        assert_eq!(state.party_max, 6);
        assert!(!game_rules_rejected_as_non_match(raw));
    }

    #[test]
    fn valid_game_rules_with_unranked_select_in_match_after_queue() {
        let state = state_from_raw(RawDeadlockState {
            game_rules_modes: Some((1, 3)),
            party_size: 1,
            ..RawDeadlockState::default()
        });

        assert_eq!(state.activity, DeadlockActivity::InMatch);
        assert_eq!(state.match_mode, MatchMode::Unranked);
        assert_eq!(state.game_mode, GameMode::Sandbox);
    }

    #[test]
    fn valid_game_rules_with_ranked_select_in_match() {
        let state = state_from_raw(RawDeadlockState {
            game_rules_modes: Some((4, 1)),
            party_size: 1,
            ..RawDeadlockState::default()
        });

        assert_eq!(state.activity, DeadlockActivity::InMatch);
        assert_eq!(state.match_mode, MatchMode::Ranked);
        assert_eq!(state.game_mode, GameMode::Normal);
    }

    #[test]
    fn valid_game_rules_with_invalid_match_mode_select_idle() {
        let raw = RawDeadlockState {
            game_rules_modes: Some((0, 3)),
            party_size: 1,
            ..RawDeadlockState::default()
        };
        let state = state_from_raw(raw);

        assert_eq!(state.activity, DeadlockActivity::Idle);
        assert_eq!(state.match_mode, MatchMode::Invalid);
        assert_eq!(state.game_mode, GameMode::Sandbox);
        assert!(game_rules_rejected_as_non_match(raw));
    }

    #[test]
    fn valid_game_rules_with_unknown_match_mode_select_idle() {
        let raw = RawDeadlockState {
            game_rules_modes: Some((99, 1)),
            party_size: 1,
            ..RawDeadlockState::default()
        };
        let state = state_from_raw(raw);

        assert_eq!(state.activity, DeadlockActivity::Idle);
        assert_eq!(state.match_mode, MatchMode::Unknown(99));
        assert_eq!(state.game_mode, GameMode::Normal);
        assert!(game_rules_rejected_as_non_match(raw));
    }

    #[test]
    fn missing_queue_and_game_rules_select_idle() {
        let state = state_from_raw(RawDeadlockState {
            party_size: 0,
            ..RawDeadlockState::default()
        });

        assert_eq!(state.activity, DeadlockActivity::Idle);
        assert_eq!(state.match_mode, MatchMode::Invalid);
        assert_eq!(state.game_mode, GameMode::Invalid);
        assert_eq!(state.party_size, 1);
    }
}
