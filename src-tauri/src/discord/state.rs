use crate::deadlock::{
    console_phase::{ConsolePhase, ConsolePhaseState, ServerKind},
    deadlock_state::{DeadlockActivity, DeadlockState, GameMode, MatchMode},
};
use std::time::{Duration, Instant};

use super::{DiscordPresenceConfig, PartyDisplay};

const TRANSITION_LOADING_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedPhase {
    MainMenu,
    Matchmaking,
    Hideout,
    ExploreNyc,
    Sandbox,
    Loading,
    TransitionLoading,
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
    pub details: String,
    pub state: String,
    pub current_hero: Option<String>,
    pub large_image: &'static str,
    pub large_text: &'static str,
    pub small_image: Option<&'static str>,
    pub small_text: Option<&'static str>,
    pub show_state: bool,
    pub show_large_image: bool,
    pub show_party: bool,
    pub party_size: u32,
    pub party_max: u32,
    pub started_at: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct PresenceModel {
    started_at: Option<i64>,
    last_known_phase: Option<ResolvedPhase>,
    transition_started_at: Option<Instant>,
}

impl PresenceModel {
    pub(crate) const fn session_started_at(&self) -> Option<i64> {
        self.started_at
    }

    pub(crate) const fn last_known_phase(&self) -> Option<ResolvedPhase> {
        self.last_known_phase
    }

    pub(crate) const fn transition_active(&self) -> bool {
        self.transition_started_at.is_some()
    }

    #[allow(dead_code)]
    pub(crate) fn observe_with_console(
        &mut self,
        enabled: bool,
        state: Option<DeadlockState>,
        console: &ConsolePhaseState,
        now: i64,
    ) -> Option<PresenceSnapshot> {
        self.observe_with_console_and_district(enabled, state, console, None, now)
    }

    pub(crate) fn observe_with_console_and_district(
        &mut self,
        enabled: bool,
        state: Option<DeadlockState>,
        console: &ConsolePhaseState,
        district_raw: Option<i8>,
        now: i64,
    ) -> Option<PresenceSnapshot> {
        self.observe_with_console_and_district_at(
            enabled,
            state,
            console,
            district_raw,
            now,
            Instant::now(),
        )
    }

    pub(crate) fn observe_with_console_and_district_at(
        &mut self,
        enabled: bool,
        state: Option<DeadlockState>,
        console: &ConsolePhaseState,
        district_raw: Option<i8>,
        now: i64,
        monotonic_now: Instant,
    ) -> Option<PresenceSnapshot> {
        self.observe_with_console_and_district_at_config(
            enabled,
            state,
            console,
            district_raw,
            now,
            monotonic_now,
            &DiscordPresenceConfig::default(),
        )
    }

    pub(crate) fn observe_with_console_and_district_at_config(
        &mut self,
        enabled: bool,
        state: Option<DeadlockState>,
        console: &ConsolePhaseState,
        district_raw: Option<i8>,
        now: i64,
        monotonic_now: Instant,
        config: &DiscordPresenceConfig,
    ) -> Option<PresenceSnapshot> {
        if !enabled || !config.global.enabled {
            self.reset();
            return None;
        }

        let state = state.unwrap_or_else(console_only_fallback_state);

        let (detected_phase, evidence) = resolve_phase(state, console);
        let resolved_phase = self.presented_phase(detected_phase, &evidence, monotonic_now);
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }

        let hero = (!console.broadcast_active)
            .then(|| console.current_hero.as_deref().and_then(hero_metadata))
            .flatten();
        let party_size = normalize_party_size(state.party_size, state.party_max);
        let party_max = state.party_max.max(1);
        let (mut details, mut presence_state) = match resolved_phase {
            ResolvedPhase::MainMenu => (
                "Deadlock".to_string(),
                format!("{}In Menu", config.main_menu.state_prefix),
            ),
            ResolvedPhase::Matchmaking => (
                "Searching for Match".to_string(),
                format!("{}Matchmaking", config.matchmaking.state_prefix),
            ),
            ResolvedPhase::Hideout => (
                config
                    .hideout
                    .use_official_hero_phrase
                    .then(|| hero.and_then(|metadata| metadata.hideout_details))
                    .flatten()
                    .unwrap_or("Deadlock")
                    .to_string(),
                format!("{}Hideout", config.hideout.state_prefix),
            ),
            ResolvedPhase::ExploreNyc => (
                config
                    .explore_nyc
                    .show_hero_in_details
                    .then(|| {
                        hero.map(|metadata| format!("Exploring NYC with {}", metadata.artwork.text))
                    })
                    .flatten()
                    .unwrap_or_else(|| "Exploring NYC".to_string()),
                config
                    .explore_nyc
                    .show_district
                    .then(|| {
                        district_raw
                            .and_then(crate::deadlock::district::district_display_name)
                            .map(|district| {
                                format!("{}{district}", config.explore_nyc.district_prefix)
                            })
                    })
                    .flatten()
                    .unwrap_or_else(|| "Explore NYC".to_string()),
            ),
            ResolvedPhase::Sandbox => {
                let hero_text = match hero {
                    Some(metadata) => format!("Practicing with {}", metadata.artwork.text),
                    None => "Practicing".to_string(),
                };
                (
                    "Sandbox".to_string(),
                    format!("{}{hero_text}", config.sandbox.state_prefix),
                )
            }
            ResolvedPhase::Loading => ("Deadlock".to_string(), "Loading...".to_string()),
            ResolvedPhase::TransitionLoading => ("Deadlock".to_string(), "Loading...".to_string()),
            ResolvedPhase::InMatch => {
                let hero_text = match hero {
                    Some(metadata) => format!("Playing as {}", metadata.artwork.text),
                    None => "Playing".to_string(),
                };
                let base = format!("{}{hero_text}", config.r#match.state_prefix);
                let presence_state = if config.r#match.party_display == PartyDisplay::Compact {
                    format!("{base} · {party_size}/{party_max}")
                } else {
                    base
                };
                ("In Match".to_string(), presence_state)
            }
            ResolvedPhase::PostMatch => (
                "Post Match".to_string(),
                format!("{}Deadlock", config.post_match.state_prefix),
            ),
            ResolvedPhase::Spectating => (
                "Spectating a game".to_string(),
                (console.broadcast_active && config.spectating.show_match_id)
                    .then(|| {
                        console
                            .spectating_match_id
                            .filter(|match_id| *match_id != 0)
                            .map(|match_id| {
                                format!("{}Match {match_id}", config.spectating.match_id_prefix)
                            })
                    })
                    .flatten()
                    .unwrap_or_default(),
            ),
        };
        if !console.broadcast_active
            && resolved_phase != ResolvedPhase::InMatch
            && party_display_for_phase(resolved_phase, config) == Some(PartyDisplay::Compact)
        {
            let party = format!("{party_size}/{party_max}");
            if resolved_phase == ResolvedPhase::Spectating {
                details.push_str(&format!(" \u{00B7} {party}"));
            } else {
                presence_state.push_str(&format!(" \u{00B7} {party}"));
            }
        }
        let artwork = artwork_for_presence(
            resolved_phase,
            console.current_hero.as_deref(),
            district_raw,
            config,
        );
        let show_large_image = match resolved_phase {
            ResolvedPhase::Loading | ResolvedPhase::TransitionLoading => {
                config.loading.use_deadlock_logo
            }
            ResolvedPhase::MainMenu => config.main_menu.use_deadlock_logo,
            _ => true,
        };
        let show_state = match resolved_phase {
            ResolvedPhase::Spectating => !presence_state.is_empty(),
            ResolvedPhase::Loading | ResolvedPhase::TransitionLoading => {
                config.loading.show_loading_state
            }
            _ => true,
        };

        Some(PresenceSnapshot {
            activity: state.activity,
            match_mode: state.match_mode,
            game_mode: state.game_mode,
            resolved_phase,
            evidence,
            details,
            state: presence_state,
            current_hero: (!console.broadcast_active
                && !matches!(
                    resolved_phase,
                    ResolvedPhase::Loading | ResolvedPhase::TransitionLoading
                ))
            .then(|| console.current_hero.clone())
            .flatten(),
            large_image: artwork.image,
            large_text: artwork.text,
            small_image: None,
            small_text: None,
            show_state,
            show_large_image,
            show_party: !console.broadcast_active && show_party_for_phase(resolved_phase, config),
            party_size,
            party_max,
            started_at: config
                .global
                .show_elapsed_time
                .then_some(self.started_at)
                .flatten(),
        })
    }

    fn reset(&mut self) {
        self.started_at = None;
        self.last_known_phase = None;
        self.transition_started_at = None;
    }

    fn presented_phase(
        &mut self,
        detected_phase: ResolvedPhase,
        evidence: &str,
        now: Instant,
    ) -> ResolvedPhase {
        let temporary_unknown_gap =
            detected_phase == ResolvedPhase::MainMenu && is_transient_menu_evidence(evidence);

        if temporary_unknown_gap {
            if let Some(started_at) = self.transition_started_at {
                if now.saturating_duration_since(started_at) < TRANSITION_LOADING_WINDOW {
                    return ResolvedPhase::TransitionLoading;
                }
                self.transition_started_at = None;
                self.last_known_phase = Some(ResolvedPhase::MainMenu);
                return ResolvedPhase::MainMenu;
            }

            if self.last_known_phase.is_some_and(is_transition_source) {
                self.transition_started_at = Some(now);
                return ResolvedPhase::TransitionLoading;
            }
        }

        self.transition_started_at = None;
        self.last_known_phase = Some(detected_phase);
        detected_phase
    }
}

fn is_transition_source(phase: ResolvedPhase) -> bool {
    phase != ResolvedPhase::MainMenu && phase != ResolvedPhase::TransitionLoading
}

fn is_transient_menu_evidence(evidence: &str) -> bool {
    evidence == "MainMenuFallback"
        || (evidence.starts_with("ServerDisconnected(") && evidence.contains("LOOPDEACTIVATE"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Artwork {
    pub image: &'static str,
    pub text: &'static str,
}

const DEADLOCK_ARTWORK: Artwork = Artwork {
    image: "deadlock_logo",
    text: "Deadlock",
};

const EXPLORE_NYC_ARTWORK: Artwork = Artwork {
    image: "explore_nyc_default",
    text: "Explore NYC",
};

struct HeroArtworkMapping {
    internal_keys: &'static [&'static str],
    artwork: Artwork,
    hideout_details: Option<&'static str>,
}

const HERO_ARTWORK_MAPPINGS: &[HeroArtworkMapping] = &[
    HeroArtworkMapping {
        internal_keys: &["abrams", "atlas_detective"],
        artwork: Artwork {
            image: "abrams",
            text: "Abrams",
        },
        hideout_details: Some("Investigating the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["apollo", "fencer"],
        artwork: Artwork {
            image: "apollo",
            text: "Apollo",
        },
        hideout_details: Some("Striving Towards Perfection in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["baba"],
        artwork: Artwork {
            image: "baba",
            text: "Baba",
        },
        hideout_details: None,
    },
    HeroArtworkMapping {
        internal_keys: &["bebop"],
        artwork: Artwork {
            image: "bebop",
            text: "Bebop",
        },
        hideout_details: Some("Ignoring Lash in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["billy", "punkgoat"],
        artwork: Artwork {
            image: "billy",
            text: "Billy",
        },
        hideout_details: Some("Ranting in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["calico", "cadence", "nano"],
        artwork: Artwork {
            image: "calico",
            text: "Calico",
        },
        hideout_details: Some("Playing With Ava in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["celeste", "unicorn"],
        artwork: Artwork {
            image: "celeste",
            text: "Celeste",
        },
        hideout_details: Some("Cleaning Up Glitter in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["deadman_danny", "deadpack"],
        artwork: Artwork {
            image: "deadman_danny",
            text: "Deadman Danny",
        },
        hideout_details: None,
    },
    HeroArtworkMapping {
        internal_keys: &["drifter"],
        artwork: Artwork {
            image: "drifter",
            text: "Drifter",
        },
        hideout_details: Some("Prowling in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["dynamo", "prof_dynamo"],
        artwork: Artwork {
            image: "dynamo",
            text: "Dynamo",
        },
        hideout_details: Some("Pontificating in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["graves", "necro"],
        artwork: Artwork {
            image: "graves",
            text: "Graves",
        },
        hideout_details: Some("Playing With The Dead in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["grey_talon", "orion", "archer"],
        artwork: Artwork {
            image: "grey_talon",
            text: "Grey Talon",
        },
        hideout_details: Some("Mourning in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["haze"],
        artwork: Artwork {
            image: "haze",
            text: "Haze",
        },
        hideout_details: Some("Sleep Walking in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["holliday", "astro"],
        artwork: Artwork {
            image: "holliday",
            text: "Holliday",
        },
        hideout_details: Some("Solving Mysteries in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["infernus", "inferno"],
        artwork: Artwork {
            image: "infernus",
            text: "Infernus",
        },
        hideout_details: Some("Mixing Drinks in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["ivy", "tengu"],
        artwork: Artwork {
            image: "ivy",
            text: "Ivy",
        },
        hideout_details: Some("Wishing the Arroyos were in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["kelvin"],
        artwork: Artwork {
            image: "kelvin",
            text: "Kelvin",
        },
        hideout_details: Some("Chilling in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["lady_geist", "geist", "ghost"],
        artwork: Artwork {
            image: "lady_geist",
            text: "Lady Geist",
        },
        hideout_details: Some("Being Fabulous in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["lash"],
        artwork: Artwork {
            image: "lash",
            text: "The Lash ™",
        },
        hideout_details: Some("Thinking About Lash in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["mcginnis", "engineer"],
        artwork: Artwork {
            image: "mcginnis",
            text: "McGinnis",
        },
        hideout_details: Some("Tinkering in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["mina", "vampirebat"],
        artwork: Artwork {
            image: "mina",
            text: "Mina",
        },
        hideout_details: Some("Grabbing a bite in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["mirage"],
        artwork: Artwork {
            image: "mirage",
            text: "Mirage",
        },
        hideout_details: Some("Dreaming of Wyoming in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["mo_and_krill", "digger"],
        artwork: Artwork {
            image: "mo_and_krill",
            text: "Mo & Krill",
        },
        hideout_details: None,
    },
    HeroArtworkMapping {
        internal_keys: &["nurse_harrow", "nurse"],
        artwork: Artwork {
            image: "nurse_harrow",
            text: "Nurse Harrow",
        },
        hideout_details: None,
    },
    HeroArtworkMapping {
        internal_keys: &["paige", "bookworm"],
        artwork: Artwork {
            image: "paige",
            text: "Paige",
        },
        hideout_details: Some("Reading in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["paradox", "chrono"],
        artwork: Artwork {
            image: "paradox",
            text: "Paradox",
        },
        hideout_details: Some("Scheming in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["pocket", "synth"],
        artwork: Artwork {
            image: "pocket",
            text: "Pocket",
        },
        hideout_details: Some("Sulking in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["rat_king", "ratking"],
        artwork: Artwork {
            image: "rat_king",
            text: "Rat King",
        },
        hideout_details: Some("Wishing the Hideout was on Long Island"),
    },
    HeroArtworkMapping {
        internal_keys: &["rem", "familiar"],
        artwork: Artwork {
            image: "rem",
            text: "Rem",
        },
        hideout_details: Some("Having Sweet Dreams in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["seven", "gigawatt"],
        artwork: Artwork {
            image: "seven",
            text: "Seven",
        },
        hideout_details: Some("Plotting in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["shiv"],
        artwork: Artwork {
            image: "shiv",
            text: "Shiv",
        },
        hideout_details: Some("Playing With Knives in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["silver", "werewolf"],
        artwork: Artwork {
            image: "silver",
            text: "Silver",
        },
        hideout_details: Some("Figuring Out What Happened Last Night in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["sinclair", "magician"],
        artwork: Artwork {
            image: "sinclair",
            text: "Sinclair",
        },
        hideout_details: Some("Working Magic in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["solomon", "chessmaster"],
        artwork: Artwork {
            image: "solomon",
            text: "Solomon",
        },
        hideout_details: None,
    },
    HeroArtworkMapping {
        internal_keys: &["the_doorman", "doorman"],
        artwork: Artwork {
            image: "the_doorman",
            text: "The Doorman",
        },
        hideout_details: Some("At Your Service in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["venator", "priest"],
        artwork: Artwork {
            image: "venator",
            text: "Venator",
        },
        hideout_details: Some("Blessing Ammunition in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["victor", "frank"],
        artwork: Artwork {
            image: "victor",
            text: "Victor",
        },
        hideout_details: Some("Brooding in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["vindicta", "hornet"],
        artwork: Artwork {
            image: "vindicta",
            text: "Vindicta",
        },
        hideout_details: Some("Brooding in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["violet", "artist"],
        artwork: Artwork {
            image: "violet",
            text: "Violet",
        },
        hideout_details: None,
    },
    HeroArtworkMapping {
        internal_keys: &["viscous"],
        artwork: Artwork {
            image: "viscous",
            text: "Viscous",
        },
        hideout_details: Some("Wishing the Hideout was The Cube"),
    },
    HeroArtworkMapping {
        internal_keys: &["vyper", "viper"],
        artwork: Artwork {
            image: "vyper",
            text: "Vyper",
        },
        hideout_details: Some("Making Pruno in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["warden"],
        artwork: Artwork {
            image: "warden",
            text: "Warden",
        },
        hideout_details: Some("Training in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["wraith"],
        artwork: Artwork {
            image: "wraith",
            text: "Wraith",
        },
        hideout_details: Some("Gambling in the Hideout"),
    },
    HeroArtworkMapping {
        internal_keys: &["yamato"],
        artwork: Artwork {
            image: "yamato",
            text: "Yamato",
        },
        hideout_details: Some("Reminiscing in the Hideout"),
    },
];

fn artwork_for_presence(
    phase: ResolvedPhase,
    hero_key: Option<&str>,
    district_raw: Option<i8>,
    config: &DiscordPresenceConfig,
) -> Artwork {
    if phase == ResolvedPhase::ExploreNyc {
        return Artwork {
            image: config
                .explore_nyc
                .show_district_image
                .then(|| district_raw.and_then(crate::deadlock::district::district_asset_key))
                .flatten()
                .unwrap_or(EXPLORE_NYC_ARTWORK.image),
            ..EXPLORE_NYC_ARTWORK
        };
    }

    let show_hero_image = match phase {
        ResolvedPhase::Hideout => config.hideout.show_hero_image,
        ResolvedPhase::Sandbox => config.sandbox.show_hero_image,
        ResolvedPhase::InMatch => config.r#match.show_hero_image,
        _ => false,
    };
    if show_hero_image {
        return hero_key.and_then(hero_artwork).unwrap_or(DEADLOCK_ARTWORK);
    }

    DEADLOCK_ARTWORK
}

fn show_party_for_phase(phase: ResolvedPhase, config: &DiscordPresenceConfig) -> bool {
    party_display_for_phase(phase, config).map_or(
        matches!(
            phase,
            ResolvedPhase::Loading | ResolvedPhase::TransitionLoading
        ),
        |display| display == PartyDisplay::Discord,
    )
}

fn party_display_for_phase(
    phase: ResolvedPhase,
    config: &DiscordPresenceConfig,
) -> Option<PartyDisplay> {
    match phase {
        ResolvedPhase::MainMenu => Some(config.main_menu.party_display),
        ResolvedPhase::Matchmaking => Some(config.matchmaking.party_display),
        ResolvedPhase::Hideout => Some(config.hideout.party_display),
        ResolvedPhase::Sandbox => Some(config.sandbox.party_display),
        ResolvedPhase::ExploreNyc => Some(config.explore_nyc.party_display),
        ResolvedPhase::InMatch => Some(config.r#match.party_display),
        ResolvedPhase::PostMatch => Some(config.post_match.party_display),
        ResolvedPhase::Spectating => None,
        ResolvedPhase::Loading | ResolvedPhase::TransitionLoading => None,
    }
}

fn hero_artwork(hero_key: &str) -> Option<Artwork> {
    hero_metadata(hero_key).map(|mapping| mapping.artwork)
}

fn hero_metadata(hero_key: &str) -> Option<&'static HeroArtworkMapping> {
    HERO_ARTWORK_MAPPINGS
        .iter()
        .find(|mapping| mapping.internal_keys.contains(&hero_key))
}

pub(crate) fn is_known_hero_key(hero_key: &str) -> bool {
    HERO_ARTWORK_MAPPINGS
        .iter()
        .any(|mapping| mapping.internal_keys.contains(&hero_key))
}

pub(crate) fn canonicalize_hero_key(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let normalized = normalized.strip_prefix("hero_").unwrap_or(&normalized);
    if normalized.is_empty() {
        return None;
    }
    if is_known_hero_key(normalized) {
        return Some(normalized.to_string());
    }

    let mut candidate = normalized;
    while let Some((prefix, _suffix)) = candidate.rsplit_once('_') {
        if is_known_hero_key(prefix) {
            return Some(prefix.to_string());
        }
        candidate = prefix;
    }

    Some(normalized.to_string())
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

    if console.broadcast_active {
        return (ResolvedPhase::Spectating, console_evidence());
    }

    let local_or_special_map = matches!(
        console.current_map.as_deref(),
        Some("dl_hideout") | Some("new_player_basics")
    ) || (console.current_map.as_deref() == Some("dl_midtown")
        && console.server_kind == ServerKind::Local);

    if console.match_found {
        return (ResolvedPhase::Loading, "GCMatchFound".to_string());
    }

    if console.matchmaking_active {
        return (ResolvedPhase::Matchmaking, "GCStartMatchmaking".to_string());
    }

    if console.current_map.as_deref() == Some("dl_hideout") {
        return (ResolvedPhase::Hideout, "Map(dl_hideout)".to_string());
    }

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

    match console.current_map.as_deref() {
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
    } else if console.phase == ConsolePhase::MainMenu {
        (ResolvedPhase::MainMenu, console_evidence())
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
                || current.current_hero != next.current_hero
                || current.large_image != next.large_image
                || current.large_text != next.large_text
                || current.small_image != next.small_image
                || current.small_text != next.small_text
                || current.show_state != next.show_state
                || current.show_large_image != next.show_large_image
                || current.show_party != next.show_party
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
            broadcast_active: false,
            spectating_match_id: None,
            matchmaking_active: false,
            matchmaking_observed: false,
            match_found: false,
            current_hero: None,
            hero_capture_mode: crate::deadlock::console_phase::HeroCaptureMode::Disabled,
            evidence: Some(evidence.to_string()),
            last_update_ms: 1,
        }
    }

    fn snapshot(memory: DeadlockState, console: &ConsolePhaseState) -> PresenceSnapshot {
        PresenceModel::default()
            .observe_with_console(true, Some(memory), console, 100)
            .unwrap()
    }

    fn snapshot_with_district(
        memory: DeadlockState,
        console: &ConsolePhaseState,
        district: Option<i8>,
    ) -> PresenceSnapshot {
        PresenceModel::default()
            .observe_with_console_and_district(true, Some(memory), console, district, 100)
            .unwrap()
    }

    fn snapshot_with_config(
        memory: DeadlockState,
        console: &ConsolePhaseState,
        config: &DiscordPresenceConfig,
    ) -> PresenceSnapshot {
        PresenceModel::default()
            .observe_with_console_and_district_at_config(
                true,
                Some(memory),
                console,
                None,
                100,
                Instant::now(),
                config,
            )
            .unwrap()
    }

    fn snapshot_at(
        model: &mut PresenceModel,
        memory: DeadlockState,
        console: &ConsolePhaseState,
        district: Option<i8>,
        unix_now: i64,
        monotonic_now: Instant,
    ) -> PresenceSnapshot {
        model
            .observe_with_console_and_district_at(
                true,
                Some(memory),
                console,
                district,
                unix_now,
                monotonic_now,
            )
            .unwrap()
    }

    fn assert_hero_mapping(raw: &str, canonical: &str, image: &'static str, text: &'static str) {
        let normalized = canonicalize_hero_key(raw);
        assert_eq!(
            normalized.as_deref(),
            Some(canonical),
            "unexpected canonical key for {raw}"
        );
        assert_eq!(
            hero_artwork(normalized.as_deref().unwrap()),
            Some(Artwork { image, text }),
            "unexpected artwork for {raw}"
        );
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
        let mut console = console_with_server(
            ConsolePhase::Unknown,
            Some("dl_midtown"),
            ServerKind::Local,
            "ChangeGameState(7)",
        );
        console.current_hero = Some("inferno".to_string());
        let result = snapshot(
            deadlock_state(
                DeadlockActivity::Idle,
                MatchMode::Invalid,
                GameMode::Invalid,
            ),
            &console,
        );
        assert_eq!(result.resolved_phase, ResolvedPhase::ExploreNyc);
        assert_eq!(result.evidence, "Map(dl_midtown)+LocalServer");
        assert_eq!(result.details, "Exploring NYC with Infernus");
        assert_eq!(result.large_image, "explore_nyc_default");
        assert_eq!(result.large_text, "Explore NYC");
        assert_eq!(result.small_image, None);
        assert_eq!(result.small_text, None);
    }

    #[test]
    fn explore_nyc_uses_only_observed_district_names() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let explore = console_with_server(
            ConsolePhase::Unknown,
            Some("dl_midtown"),
            ServerKind::Local,
            "Map(dl_midtown)",
        );

        let plaza = snapshot_with_district(memory, &explore, Some(8));
        assert_eq!(plaza.state, "› Plaza · 2/6");
        assert_eq!(plaza.large_image, "plaza");

        let docks = snapshot_with_district(memory, &explore, Some(3));
        assert_eq!(docks.state, "› York : Docks · 2/6");
        assert_eq!(docks.large_image, "york_docks");

        let known = snapshot_with_district(memory, &explore, Some(13));
        assert_eq!(known.resolved_phase, ResolvedPhase::ExploreNyc);
        assert_eq!(
            (known.details.as_str(), known.state.as_str()),
            ("Exploring NYC", "› York : Factory · 2/6")
        );
        assert_eq!(known.large_image, "york_factory");

        let uptown = snapshot_with_district(memory, &explore, Some(15));
        assert_eq!(uptown.state, "› Broadway : Uptown · 2/6");
        assert_eq!(uptown.large_image, "broadway_uptown");
        assert!(presence_changed(Some(&docks), Some(&uptown)));

        for unknown in [None, Some(0), Some(20), Some(127)] {
            let fallback = snapshot_with_district(memory, &explore, unknown);
            assert_eq!(
                (fallback.details.as_str(), fallback.state.as_str()),
                ("Exploring NYC", "Explore NYC · 2/6")
            );
            assert_eq!(fallback.large_image, "explore_nyc_default");
            assert!(presence_changed(Some(&known), Some(&fallback)));
        }
    }

    #[test]
    fn explore_nyc_reuses_the_known_hero_display_name_and_district_artwork() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut explore = console_with_server(
            ConsolePhase::Unknown,
            Some("dl_midtown"),
            ServerKind::Local,
            "Map(dl_midtown)",
        );
        explore.current_hero = Some("ratking".to_string());

        let docks = snapshot_with_district(memory, &explore, Some(3));
        assert_eq!(docks.details, "Exploring NYC with Rat King");
        assert_eq!(docks.state, "› York : Docks · 2/6");
        assert_eq!(docks.large_image, "york_docks");

        let factory = snapshot_with_district(memory, &explore, Some(13));
        assert_eq!(factory.details, "Exploring NYC with Rat King");
        assert_eq!(factory.state, "› York : Factory · 2/6");
        assert_eq!(factory.large_image, "york_factory");
        assert!(presence_changed(Some(&docks), Some(&factory)));

        explore.current_hero = Some("artist".to_string());
        let violet = snapshot_with_district(memory, &explore, Some(13));
        assert_eq!(violet.details, "Exploring NYC with Violet");
        assert_eq!(violet.large_image, "york_factory");
        assert!(presence_changed(Some(&factory), Some(&violet)));
    }

    #[test]
    fn explore_nyc_without_a_known_hero_keeps_the_existing_fallback() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut explore = console_with_server(
            ConsolePhase::Unknown,
            Some("dl_midtown"),
            ServerKind::Local,
            "Map(dl_midtown)",
        );

        for hero in [None, Some("unknown_hero")] {
            explore.current_hero = hero.map(str::to_string);
            let result = snapshot_with_district(memory, &explore, Some(3));
            assert_eq!(result.details, "Exploring NYC");
            assert!(!result.details.contains("Unknown"));
        }
    }

    #[test]
    fn districts_are_ignored_outside_explore_nyc() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let cases = [
            (
                console_with_server(
                    ConsolePhase::InMatch,
                    Some("dl_midtown"),
                    ServerKind::Remote,
                    "ChangeGameState(7)",
                ),
                ResolvedPhase::InMatch,
                ("In Match", "Playing · 2/6"),
            ),
            (
                console_with_server(
                    ConsolePhase::Hideout,
                    Some("dl_hideout"),
                    ServerKind::Local,
                    "Map(dl_hideout)",
                ),
                ResolvedPhase::Hideout,
                ("Deadlock", "Hideout · 2/6"),
            ),
            (
                console_with_server(
                    ConsolePhase::Spectating,
                    Some("dl_midtown"),
                    ServerKind::Remote,
                    "PlayingBroadcast",
                ),
                ResolvedPhase::Spectating,
                ("Spectating a game", ""),
            ),
        ];

        for (console, phase, text) in cases {
            let result = snapshot_with_district(memory, &console, Some(13));
            assert_eq!(result.resolved_phase, phase);
            assert_eq!((result.details.as_str(), result.state.as_str()), text);
            assert_eq!(result.large_image, "deadlock_logo");
        }
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
                (result.details.as_str(), result.state.as_str()),
                ("Searching for Match", "Matchmaking · 2/6")
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
            assert_eq!(result.state, "Playing · 2/6");
            assert!(!result.show_party);
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
        assert_eq!(
            (result.details.as_str(), result.state.as_str()),
            ("Sandbox", "Practicing · 2/6")
        );
        assert_eq!(result.large_image, "deadlock_logo");
        assert!(!result.show_party);
    }

    #[test]
    fn sandbox_uses_known_hero_prefix_party_mode_and_artwork_config() {
        let memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Unranked,
            GameMode::Sandbox,
        );
        let mut sandbox = console(
            ConsolePhase::InMatch,
            Some("new_player_basics"),
            "Map(new_player_basics)",
        );
        sandbox.current_hero = Some("haze".to_string());

        let compact = snapshot(memory, &sandbox);
        assert_eq!(compact.details, "Sandbox");
        assert_eq!(compact.state, "Practicing with Haze · 2/6");
        assert_eq!(compact.large_image, "haze");
        assert!(!compact.show_party);

        let mut prefixed_config = DiscordPresenceConfig::default();
        prefixed_config.sandbox.state_prefix = "› ".to_string();
        let prefixed = snapshot_with_config(memory, &sandbox, &prefixed_config);
        assert_eq!(prefixed.state, "› Practicing with Haze · 2/6");

        let mut discord_config = DiscordPresenceConfig::default();
        discord_config.sandbox.party_display = PartyDisplay::Discord;
        let discord = snapshot_with_config(memory, &sandbox, &discord_config);
        assert_eq!(discord.state, "Practicing with Haze");
        assert!(discord.show_party);

        let mut hidden_config = DiscordPresenceConfig::default();
        hidden_config.sandbox.party_display = PartyDisplay::Hidden;
        let hidden = snapshot_with_config(memory, &sandbox, &hidden_config);
        assert_eq!(hidden.state, "Practicing with Haze");
        assert!(!hidden.show_party);

        let mut hidden_image_config = DiscordPresenceConfig::default();
        hidden_image_config.sandbox.show_hero_image = false;
        let hidden_image = snapshot_with_config(memory, &sandbox, &hidden_image_config);
        assert_eq!(hidden_image.large_image, "deadlock_logo");

        sandbox.current_hero = Some("unknown_hero".to_string());
        let unknown = snapshot_with_config(memory, &sandbox, &prefixed_config);
        assert_eq!(unknown.state, "› Practicing · 2/6");
        assert_eq!(unknown.large_image, "deadlock_logo");
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
        assert_eq!(result.details, "Spectating a game");
        assert_eq!(result.state, "");
        assert!(!result.show_state);
        assert_eq!((result.party_size, result.party_max), (2, 6));
        assert_eq!(result.started_at, Some(100));
    }

    #[test]
    fn broadcast_spectating_shows_optional_prefixed_match_id_without_party() {
        let mut console = console_with_server(
            ConsolePhase::Spectating,
            Some("dl_midtown"),
            ServerKind::Local,
            "PlayingBroadcast",
        );
        console.broadcast_active = true;
        console.spectating_match_id = Some(110501755);
        console.current_hero = Some("bookworm".to_string());

        let memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Ranked,
            GameMode::Normal,
        );
        let mut config = DiscordPresenceConfig::default();
        config.spectating.match_id_prefix = "› ".to_string();
        let result = snapshot_with_config(memory, &console, &config);
        assert_eq!(result.resolved_phase, ResolvedPhase::Spectating);
        assert_eq!(result.details, "Spectating a game");
        assert_eq!(result.state, "› Match 110501755");
        assert!(result.show_state);
        assert_eq!(result.current_hero, None);
        assert_eq!(result.large_image, "deadlock_logo");
        assert!(!result.show_party);

        let mut hidden_id = DiscordPresenceConfig::default();
        hidden_id.spectating.show_match_id = false;
        let result = snapshot_with_config(memory, &console, &hidden_id);
        assert_eq!(result.state, "");
        assert!(!result.show_state);

        console.spectating_match_id = None;
        let result = snapshot_with_config(memory, &console, &DiscordPresenceConfig::default());
        assert_eq!(result.state, "");
        assert!(!result.show_state);

        console.spectating_match_id = Some(0);
        let result = snapshot_with_config(memory, &console, &DiscordPresenceConfig::default());
        assert_eq!(result.state, "");
        assert!(!result.show_state);
    }

    #[test]
    fn configured_prefixes_preserve_details_and_native_or_hidden_party_modes() {
        let idle = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut config = DiscordPresenceConfig::default();
        config.hideout.state_prefix = "H: ".to_string();
        config.hideout.party_display = PartyDisplay::Discord;
        config.main_menu.state_prefix = "M: ".to_string();
        config.main_menu.party_display = PartyDisplay::Discord;
        config.matchmaking.state_prefix = "Q: ".to_string();
        config.matchmaking.party_display = PartyDisplay::Discord;
        config.r#match.state_prefix = "G: ".to_string();
        config.r#match.party_display = PartyDisplay::Discord;
        config.post_match.state_prefix = "P: ".to_string();
        config.post_match.party_display = PartyDisplay::Discord;

        let mut hideout = console_with_server(
            ConsolePhase::Hideout,
            Some("dl_hideout"),
            ServerKind::Local,
            "Map(dl_hideout)",
        );
        hideout.current_hero = Some("priest".to_string());
        let hideout = snapshot_with_config(idle, &hideout, &config);
        assert_eq!(hideout.details, "Blessing Ammunition in the Hideout");
        assert_eq!(hideout.state, "H: Hideout");
        assert!(hideout.show_party);

        let menu = snapshot_with_config(
            idle,
            &console(ConsolePhase::MainMenu, None, "LoopMode(menu)"),
            &config,
        );
        assert_eq!(
            (menu.details.as_str(), menu.state.as_str()),
            ("Deadlock", "M: In Menu")
        );
        assert!(menu.show_party);

        let matchmaking = snapshot_with_config(
            idle,
            &active_matchmaking_console("dl_hideout", ServerKind::Local),
            &config,
        );
        assert_eq!(matchmaking.details, "Searching for Match");
        assert_eq!(matchmaking.state, "Q: Matchmaking");
        assert!(matchmaking.show_party);

        let mut match_console = console_with_server(
            ConsolePhase::InMatch,
            Some("dl_midtown"),
            ServerKind::Remote,
            "ChangeGameState(7)",
        );
        match_console.current_hero = Some("priest".to_string());
        let in_match = snapshot_with_config(
            deadlock_state(
                DeadlockActivity::InMatch,
                MatchMode::Unranked,
                GameMode::Normal,
            ),
            &match_console,
            &config,
        );
        assert_eq!(in_match.details, "In Match");
        assert_eq!(in_match.state, "G: Playing as Venator");
        assert!(in_match.show_party);

        let post_match = snapshot_with_config(
            idle,
            &console(ConsolePhase::PostMatch, None, "ChangeGameState(6)"),
            &config,
        );
        assert_eq!(post_match.details, "Post Match");
        assert_eq!(post_match.state, "P: Deadlock");
        assert!(post_match.show_party);

        config.r#match.party_display = PartyDisplay::Hidden;
        let hidden = snapshot_with_config(
            deadlock_state(
                DeadlockActivity::InMatch,
                MatchMode::Unranked,
                GameMode::Normal,
            ),
            &match_console,
            &config,
        );
        assert_eq!(hidden.state, "G: Playing as Venator");
        assert!(!hidden.show_party);
    }

    #[test]
    fn converted_contexts_use_inline_compact_party_text() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut config = DiscordPresenceConfig::default();
        config.hideout.party_display = PartyDisplay::Compact;
        config.explore_nyc.party_display = PartyDisplay::Compact;
        config.main_menu.party_display = PartyDisplay::Compact;
        config.matchmaking.party_display = PartyDisplay::Compact;
        config.post_match.party_display = PartyDisplay::Compact;
        config.hideout.state_prefix = "H: ".to_string();
        config.main_menu.state_prefix = "M: ".to_string();
        config.matchmaking.state_prefix = "Q: ".to_string();
        config.post_match.state_prefix = "P: ".to_string();

        let cases = [
            (
                console(ConsolePhase::MainMenu, None, "LoopMode(menu)"),
                None,
                "M: In Menu \u{00B7} 2/6",
            ),
            (
                console_with_server(
                    ConsolePhase::Hideout,
                    Some("dl_hideout"),
                    ServerKind::Local,
                    "Map(dl_hideout)",
                ),
                None,
                "H: Hideout \u{00B7} 2/6",
            ),
            (
                console_with_server(
                    ConsolePhase::Unknown,
                    Some("dl_midtown"),
                    ServerKind::Local,
                    "Map(dl_midtown)",
                ),
                Some(8),
                "\u{203A} Plaza \u{00B7} 2/6",
            ),
            (
                active_matchmaking_console("dl_hideout", ServerKind::Local),
                None,
                "Q: Matchmaking \u{00B7} 2/6",
            ),
            (
                console(ConsolePhase::PostMatch, None, "ChangeGameState(6)"),
                None,
                "P: Deadlock \u{00B7} 2/6",
            ),
        ];

        for (console, district, expected_state) in cases {
            let result = PresenceModel::default()
                .observe_with_console_and_district_at_config(
                    true,
                    Some(memory),
                    &console,
                    district,
                    100,
                    Instant::now(),
                    &config,
                )
                .unwrap();
            assert_eq!(result.state, expected_state);
            assert!(!result.show_party);
        }

        config.r#match.state_prefix = "G: ".to_string();
        let in_match = snapshot_with_config(
            deadlock_state(
                DeadlockActivity::InMatch,
                MatchMode::Unranked,
                GameMode::Normal,
            ),
            &console_with_server(
                ConsolePhase::InMatch,
                Some("dl_midtown"),
                ServerKind::Remote,
                "ChangeGameState(7)",
            ),
            &config,
        );
        assert_eq!(in_match.details, "In Match");
        assert_eq!(in_match.state, "G: Playing \u{00B7} 2/6");
        assert!(!in_match.show_party);
    }

    #[test]
    fn global_preview_prefix_is_not_applied_at_runtime_and_loading_stays_clean() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut config = DiscordPresenceConfig::default();
        config.global.preview_prefix = "Preview: ".to_string();
        config.hideout.state_prefix = "H: ".to_string();
        config.main_menu.party_display = PartyDisplay::Hidden;

        let menu = snapshot_with_config(
            memory,
            &console(ConsolePhase::MainMenu, None, "LoopMode(menu)"),
            &config,
        );
        assert_eq!(menu.state, "In Menu");

        let loading = snapshot_with_config(
            memory,
            &console(ConsolePhase::Loading, None, "GCMatchFound"),
            &config,
        );
        assert_eq!(loading.details, "Deadlock");
        assert_eq!(loading.state, "Loading...");
        assert_eq!(loading.large_image, "deadlock_logo");
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
            (result.details.as_str(), result.state.as_str()),
            ("Deadlock", "Loading...")
        );
        assert_eq!(result.large_image, "deadlock_logo");
        assert_eq!(result.current_hero, None);
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
        assert_eq!(result.started_at, Some(100));
        assert_eq!(result.large_image, "deadlock_logo");
        assert_eq!(result.large_text, "Deadlock");
        assert_eq!(result.small_image, None);
        assert_eq!(result.small_text, None);
    }

    #[test]
    fn temporary_unknown_gap_after_known_phase_uses_clean_loading_presence() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let start = Instant::now();
        let mut model = PresenceModel::default();
        let mut explore = console_with_server(
            ConsolePhase::Unknown,
            Some("dl_midtown"),
            ServerKind::Local,
            "Map(dl_midtown)",
        );
        explore.current_hero = Some("inferno".to_string());
        snapshot_at(&mut model, memory, &explore, Some(3), 100, start);

        let mut gap = ConsolePhaseState::default();
        gap.current_hero = Some("inferno".to_string());
        let loading = snapshot_at(
            &mut model,
            memory,
            &gap,
            Some(3),
            101,
            start + Duration::from_secs(1),
        );

        assert_eq!(loading.resolved_phase, ResolvedPhase::TransitionLoading);
        assert_eq!(
            (loading.details.as_str(), loading.state.as_str()),
            ("Deadlock", "Loading...")
        );
        assert_eq!(loading.large_image, "deadlock_logo");
        assert_eq!(loading.large_text, "Deadlock");
        assert_eq!(loading.current_hero, None);
        assert_eq!(loading.started_at, Some(100));
        assert_ne!(loading.large_image, "?");
        assert_ne!(loading.details, "Mixing Drinks in the Hideout");
    }

    #[test]
    fn valid_phase_immediately_ends_transition_loading() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let start = Instant::now();
        let mut model = PresenceModel::default();
        let explore = console_with_server(
            ConsolePhase::Unknown,
            Some("dl_midtown"),
            ServerKind::Local,
            "Map(dl_midtown)",
        );
        snapshot_at(&mut model, memory, &explore, Some(3), 100, start);
        let loading = snapshot_at(
            &mut model,
            memory,
            &ConsolePhaseState::default(),
            None,
            101,
            start + Duration::from_secs(1),
        );
        assert_eq!(loading.resolved_phase, ResolvedPhase::TransitionLoading);

        let mut hideout = console_with_server(
            ConsolePhase::Hideout,
            Some("dl_hideout"),
            ServerKind::Local,
            "Map(dl_hideout)",
        );
        hideout.current_hero = Some("inferno".to_string());
        let resolved = snapshot_at(
            &mut model,
            memory,
            &hideout,
            None,
            102,
            start + Duration::from_secs(2),
        );
        assert_eq!(resolved.resolved_phase, ResolvedPhase::Hideout);
        assert_eq!(resolved.details, "Mixing Drinks in the Hideout");
        assert_eq!(resolved.state, "Hideout · 2/6");
        assert_eq!(resolved.large_image, "infernus");
        assert_eq!(resolved.started_at, Some(100));
    }

    #[test]
    fn hideout_map_overrides_stale_loading_phase_immediately() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let start = Instant::now();
        let mut model = PresenceModel::default();
        let match_console = console_with_server(
            ConsolePhase::InMatch,
            None,
            ServerKind::Remote,
            "ChangeGameState(7)",
        );
        snapshot_at(&mut model, memory, &match_console, None, 100, start);

        let gap = console(
            ConsolePhase::MainMenu,
            None,
            "ServerDisconnected(NETWORK_DISCONNECT_LOOPDEACTIVATE)",
        );
        let loading = snapshot_at(
            &mut model,
            memory,
            &gap,
            None,
            101,
            start + Duration::from_secs(1),
        );
        assert_eq!(loading.resolved_phase, ResolvedPhase::TransitionLoading);

        let mut hideout = console_with_server(
            ConsolePhase::Loading,
            Some("dl_hideout"),
            ServerKind::Local,
            "StaleLoadingAfterMap",
        );
        hideout.current_hero = Some("ratking".to_string());
        let resolved = snapshot_at(
            &mut model,
            memory,
            &hideout,
            None,
            102,
            start + Duration::from_millis(1_100),
        );

        assert_eq!(resolved.resolved_phase, ResolvedPhase::Hideout);
        assert_eq!(resolved.details, "Wishing the Hideout was on Long Island");
        assert_eq!(resolved.state, "Hideout · 2/6");
        assert_eq!(resolved.large_image, "rat_king");
        assert!(!model.transition_active());
    }

    #[test]
    fn transition_loading_expires_after_exactly_five_seconds() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let start = Instant::now();
        let mut model = PresenceModel::default();
        let hideout = console(ConsolePhase::Hideout, None, "HeroTest");
        snapshot_at(&mut model, memory, &hideout, None, 100, start);

        let gap = ConsolePhaseState::default();
        let loading = snapshot_at(
            &mut model,
            memory,
            &gap,
            None,
            101,
            start + Duration::from_secs(1),
        );
        assert_eq!(loading.resolved_phase, ResolvedPhase::TransitionLoading);
        let still_loading = snapshot_at(
            &mut model,
            memory,
            &gap,
            None,
            105,
            start + Duration::from_millis(5_999),
        );
        assert_eq!(
            still_loading.resolved_phase,
            ResolvedPhase::TransitionLoading
        );
        let expired = snapshot_at(
            &mut model,
            memory,
            &gap,
            None,
            106,
            start + Duration::from_secs(6),
        );
        assert_eq!(expired.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(
            (expired.details.as_str(), expired.state.as_str()),
            ("Deadlock", "In Menu · 2/6")
        );
    }

    #[test]
    fn startup_unknown_and_explicit_menu_never_trigger_transition_loading() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let start = Instant::now();
        let mut model = PresenceModel::default();
        let startup = snapshot_at(
            &mut model,
            memory,
            &ConsolePhaseState::default(),
            None,
            100,
            start,
        );
        assert_eq!(startup.resolved_phase, ResolvedPhase::MainMenu);

        let hideout = console(ConsolePhase::Hideout, None, "HeroTest");
        snapshot_at(
            &mut model,
            memory,
            &hideout,
            None,
            101,
            start + Duration::from_secs(1),
        );
        let menu = console(ConsolePhase::MainMenu, None, "MainMenu");
        let stable_menu = snapshot_at(
            &mut model,
            memory,
            &menu,
            None,
            102,
            start + Duration::from_secs(2),
        );
        assert_eq!(stable_menu.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(stable_menu.state, "In Menu · 2/6");
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
    fn artwork_changes_republish_presence() {
        let first = snapshot(
            deadlock_state(
                DeadlockActivity::Idle,
                MatchMode::Invalid,
                GameMode::Invalid,
            ),
            &ConsolePhaseState::default(),
        );
        let mut second = first.clone();
        second.large_image = "infernus";
        second.large_text = "Infernus";

        assert!(presence_changed(Some(&first), Some(&second)));

        let mut inferno_console = console(ConsolePhase::Hideout, None, "HeroSwap");
        inferno_console.current_hero = Some("inferno".to_string());
        let mut ivy_console = inferno_console.clone();
        ivy_console.current_hero = Some("ivy".to_string());
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let infernus = snapshot(memory, &inferno_console);
        let ivy = snapshot(memory, &ivy_console);
        assert!(presence_changed(Some(&infernus), Some(&ivy)));
    }

    #[test]
    fn curated_hero_aliases_map_to_uploaded_assets_and_names() {
        for (raw, canonical, image, text) in [
            ("artist", "artist", "violet", "Violet"),
            ("baba", "baba", "baba", "Baba"),
            ("chessmaster", "chessmaster", "solomon", "Solomon"),
            ("deadpack", "deadpack", "deadman_danny", "Deadman Danny"),
            ("nurse", "nurse", "nurse_harrow", "Nurse Harrow"),
            ("ratking", "ratking", "rat_king", "Rat King"),
            ("bookworm", "bookworm", "paige", "Paige"),
            ("fencer", "fencer", "apollo", "Apollo"),
            ("frank", "frank", "victor", "Victor"),
            ("frank_v2", "frank", "victor", "Victor"),
            ("familiar", "familiar", "rem", "Rem"),
            ("necro", "necro", "graves", "Graves"),
            ("priest", "priest", "venator", "Venator"),
            ("punkgoat", "punkgoat", "billy", "Billy"),
            ("unicorn", "unicorn", "celeste", "Celeste"),
            ("vampirebat", "vampirebat", "mina", "Mina"),
            ("viper", "viper", "vyper", "Vyper"),
            ("doorman", "doorman", "the_doorman", "The Doorman"),
            ("doorman_v2", "doorman", "the_doorman", "The Doorman"),
            ("archer", "archer", "grey_talon", "Grey Talon"),
            ("archer_v2", "archer", "grey_talon", "Grey Talon"),
            ("gigawatt", "gigawatt", "seven", "Seven"),
            ("gigawatt_prisoner", "gigawatt", "seven", "Seven"),
            ("kelvin_explorer", "kelvin", "kelvin", "Kelvin"),
            ("prof_dynamo", "prof_dynamo", "dynamo", "Dynamo"),
            ("synth", "synth", "pocket", "Pocket"),
            ("tengu", "tengu", "ivy", "Ivy"),
            ("wraith_gen_man", "wraith", "wraith", "Wraith"),
            ("wraith_magician", "wraith", "wraith", "Wraith"),
            ("wraith_puppeteer", "wraith", "wraith", "Wraith"),
            ("yamato_v2", "yamato", "yamato", "Yamato"),
            ("atlas_detective_v2", "atlas_detective", "abrams", "Abrams"),
            ("engineer", "engineer", "mcginnis", "McGinnis"),
            ("ghost", "ghost", "lady_geist", "Lady Geist"),
            ("nano", "nano", "calico", "Calico"),
            ("inferno_v4", "inferno", "infernus", "Infernus"),
            ("hornet_v3", "hornet", "vindicta", "Vindicta"),
            ("shiv_ult", "shiv", "shiv", "Shiv"),
            ("cadence", "cadence", "calico", "Calico"),
            ("orion", "orion", "grey_talon", "Grey Talon"),
            ("silver", "silver", "silver", "Silver"),
            ("lash", "lash", "lash", "The Lash ™"),
        ] {
            assert_hero_mapping(raw, canonical, image, text);
        }

        assert_eq!(hero_artwork("unknown_hero"), None);
        assert_eq!(
            artwork_for_presence(
                ResolvedPhase::Hideout,
                Some("unknown_hero"),
                Some(13),
                &DiscordPresenceConfig::default(),
            ),
            DEADLOCK_ARTWORK
        );
    }

    #[test]
    fn model_variants_canonicalize_to_the_longest_known_internal_key() {
        for (raw, expected) in [
            ("gigawatt_prisoner", "gigawatt"),
            ("hero_gigawatt_prisoner", "gigawatt"),
            ("mirage_v2", "mirage"),
            ("gigawatt", "gigawatt"),
            ("lady_geist_variant", "lady_geist"),
            ("totally_unknown_model", "totally_unknown_model"),
        ] {
            assert_eq!(
                canonicalize_hero_key(raw).as_deref(),
                Some(expected),
                "unexpected canonical key for {raw}"
            );
        }
    }

    #[test]
    fn production_hero_mapping_only_references_uploaded_assets() {
        const UPLOADED_ASSETS: &[&str] = &[
            "abrams",
            "apollo",
            "baba",
            "bebop",
            "billy",
            "calico",
            "celeste",
            "deadman_danny",
            "drifter",
            "dynamo",
            "graves",
            "grey_talon",
            "haze",
            "holliday",
            "infernus",
            "ivy",
            "kelvin",
            "lady_geist",
            "lash",
            "mcginnis",
            "mina",
            "mirage",
            "mo_and_krill",
            "nurse_harrow",
            "paige",
            "paradox",
            "pocket",
            "rat_king",
            "rem",
            "seven",
            "shiv",
            "silver",
            "sinclair",
            "solomon",
            "the_doorman",
            "venator",
            "victor",
            "vindicta",
            "violet",
            "viscous",
            "vyper",
            "warden",
            "wraith",
            "yamato",
        ];

        for mapping in HERO_ARTWORK_MAPPINGS {
            assert!(
                UPLOADED_ASSETS.contains(&mapping.artwork.image),
                "unknown Discord asset {}",
                mapping.artwork.image
            );
        }
        for asset in UPLOADED_ASSETS {
            assert!(
                HERO_ARTWORK_MAPPINGS
                    .iter()
                    .any(|mapping| mapping.artwork.image == *asset),
                "uploaded Discord asset {asset} has no mapping"
            );
        }
    }

    #[test]
    fn known_hero_artwork_remains_available_in_hideout_and_in_match() {
        for phase in [ConsolePhase::Hideout, ConsolePhase::InMatch] {
            let mut console = console(phase, None, "HeroTest");
            console.current_hero = Some("inferno".to_string());
            let result = snapshot(
                deadlock_state(
                    DeadlockActivity::Idle,
                    MatchMode::Invalid,
                    GameMode::Invalid,
                ),
                &console,
            );

            assert_eq!(result.current_hero.as_deref(), Some("inferno"));
            assert_eq!(result.large_image, "infernus");
            assert_eq!(result.large_text, "Infernus");
            if phase == ConsolePhase::InMatch {
                assert_eq!(result.details, "In Match");
                assert_eq!(result.state, "Playing as Infernus · 2/6");
                assert!(!result.show_party);
            }
        }
    }

    #[test]
    fn match_hero_text_and_artwork_have_clean_configured_fallbacks() {
        let mut memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Unranked,
            GameMode::Normal,
        );
        memory.party_size = 1;
        let mut known = console_with_server(
            ConsolePhase::InMatch,
            None,
            ServerKind::Remote,
            "ChangeGameState(7)",
        );
        known.current_hero = Some("ratking".to_string());

        let shown = snapshot(memory, &known);
        assert_eq!(
            (shown.details.as_str(), shown.state.as_str()),
            ("In Match", "Playing as Rat King · 1/6")
        );
        assert_eq!(shown.current_hero.as_deref(), Some("ratking"));
        assert_eq!(shown.large_image, "rat_king");
        assert!(!shown.show_party);

        let mut hidden_image_config = DiscordPresenceConfig::default();
        hidden_image_config.r#match.show_hero_image = false;
        let hidden_image = snapshot_with_config(memory, &known, &hidden_image_config);
        assert_eq!(hidden_image.state, "Playing as Rat King · 1/6");
        assert_eq!(hidden_image.large_image, "deadlock_logo");

        let mut three_players = memory;
        three_players.party_size = 3;
        let compact = snapshot(three_players, &known);
        assert_eq!(compact.state, "Playing as Rat King · 3/6");
        assert!(!compact.show_party);

        let mut discord_config = DiscordPresenceConfig::default();
        discord_config.r#match.party_display = PartyDisplay::Discord;
        let discord = snapshot_with_config(three_players, &known, &discord_config);
        assert_eq!(discord.state, "Playing as Rat King");
        assert!(discord.show_party);
        assert_eq!((discord.party_size, discord.party_max), (3, 6));

        let mut hidden_config = DiscordPresenceConfig::default();
        hidden_config.r#match.party_display = PartyDisplay::Hidden;
        let hidden = snapshot_with_config(three_players, &known, &hidden_config);
        assert_eq!(hidden.state, "Playing as Rat King");
        assert!(!hidden.show_party);

        for hero in [None, Some("unknown_hero")] {
            let mut unknown = known.clone();
            unknown.current_hero = hero.map(str::to_string);
            let fallback = snapshot(memory, &unknown);
            assert_eq!(
                (fallback.details.as_str(), fallback.state.as_str()),
                ("In Match", "Playing · 1/6")
            );
            assert_eq!(fallback.large_image, "deadlock_logo");
        }
    }

    #[test]
    fn hideout_uses_official_english_hero_details_and_updates_on_hero_change() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut infernus_console = console(ConsolePhase::Hideout, None, "HeroTest");
        infernus_console.current_hero = Some("inferno".to_string());
        let mut ivy_console = infernus_console.clone();
        ivy_console.current_hero = Some("tengu".to_string());

        let infernus = snapshot(memory, &infernus_console);
        assert_eq!(infernus.details, "Mixing Drinks in the Hideout");
        assert_eq!(infernus.state, "Hideout · 2/6");
        assert_eq!(infernus.large_image, "infernus");

        let ivy = snapshot(memory, &ivy_console);
        assert_eq!(ivy.details, "Wishing the Arroyos were in the Hideout");
        assert_eq!(ivy.state, "Hideout · 2/6");
        assert_eq!(ivy.large_image, "ivy");
        assert!(presence_changed(Some(&infernus), Some(&ivy)));
    }

    #[test]
    fn hideout_details_fall_back_when_hero_or_official_phrase_is_unknown() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        for hero in [None, Some("baba"), Some("unknown_hero")] {
            let mut hideout = console(ConsolePhase::Hideout, None, "HeroTest");
            hideout.current_hero = hero.map(str::to_string);
            let result = snapshot(memory, &hideout);
            assert_eq!(result.details, "Deadlock");
            assert_eq!(result.state, "Hideout · 2/6");
            if hero == Some("baba") {
                assert_eq!(result.large_image, "baba");
            }
        }
    }

    #[test]
    fn hideout_details_never_leak_into_other_phases() {
        let memory = deadlock_state(
            DeadlockActivity::InMatch,
            MatchMode::Unranked,
            GameMode::Normal,
        );
        for (mut console, expected_details) in [
            (
                console_with_server(
                    ConsolePhase::Unknown,
                    Some("dl_midtown"),
                    ServerKind::Local,
                    "Map(dl_midtown)",
                ),
                "Exploring NYC with Infernus",
            ),
            (
                console_with_server(
                    ConsolePhase::InMatch,
                    Some("dl_midtown"),
                    ServerKind::Remote,
                    "ChangeGameState(7)",
                ),
                "In Match",
            ),
            (
                console(
                    ConsolePhase::Spectating,
                    Some("dl_midtown"),
                    "PlayingBroadcast",
                ),
                "Spectating a game",
            ),
        ] {
            console.current_hero = Some("inferno".to_string());
            let result = snapshot(memory, &console);
            assert_eq!(result.details, expected_details);
            assert_ne!(result.details, "Mixing Drinks in the Hideout");
        }
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
    fn hero_and_artwork_changes_do_not_reset_the_session_timestamp() {
        let memory = deadlock_state(
            DeadlockActivity::Idle,
            MatchMode::Invalid,
            GameMode::Invalid,
        );
        let mut inferno = console(ConsolePhase::Hideout, None, "HeroTest");
        inferno.current_hero = Some("inferno".to_string());
        let mut ivy = inferno.clone();
        ivy.current_hero = Some("ivy".to_string());

        let mut model = PresenceModel::default();
        let first = model
            .observe_with_console(true, Some(memory), &inferno, 100)
            .unwrap();
        let repeated = model
            .observe_with_console(true, Some(memory), &inferno, 500)
            .unwrap();
        let hero_changed = model
            .observe_with_console(true, Some(memory), &ivy, 900)
            .unwrap();

        assert_eq!(first.started_at, Some(100));
        assert_eq!(repeated.started_at, Some(100));
        assert_eq!(hero_changed.started_at, Some(100));
        assert_eq!(hero_changed.large_image, "ivy");
    }

    #[test]
    fn phase_changes_reuse_the_session_timestamp() {
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

        assert_eq!(first.started_at, Some(100));
        assert_eq!(second.started_at, Some(100));
    }

    #[test]
    fn returning_to_main_menu_preserves_the_session_timestamp() {
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

        let menu_console = console(ConsolePhase::MainMenu, None, "MainMenu");
        let menu = model
            .observe_with_console(
                true,
                Some(deadlock_state(
                    DeadlockActivity::Idle,
                    MatchMode::Invalid,
                    GameMode::Invalid,
                )),
                &menu_console,
                200,
            )
            .unwrap();
        assert_eq!(menu.resolved_phase, ResolvedPhase::MainMenu);
        assert_eq!(menu.started_at, Some(100));
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
