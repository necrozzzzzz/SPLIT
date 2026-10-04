use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordPresenceConfig {
    pub global: GlobalPresenceConfig,
    pub hideout: HideoutPresenceConfig,
    pub sandbox: SandboxPresenceConfig,
    pub explore_nyc: ExploreNycPresenceConfig,
    pub loading: LoadingPresenceConfig,
    pub main_menu: MainMenuPresenceConfig,
    pub matchmaking: PartyPresenceConfig,
    #[serde(rename = "match")]
    pub r#match: MatchPresenceConfig,
    pub spectating: SpectatingPresenceConfig,
    pub post_match: PartyPresenceConfig,
}

impl Default for DiscordPresenceConfig {
    fn default() -> Self {
        Self {
            global: GlobalPresenceConfig::default(),
            hideout: HideoutPresenceConfig::default(),
            sandbox: SandboxPresenceConfig::default(),
            explore_nyc: ExploreNycPresenceConfig::default(),
            loading: LoadingPresenceConfig::default(),
            main_menu: MainMenuPresenceConfig::default(),
            matchmaking: PartyPresenceConfig::default(),
            r#match: MatchPresenceConfig::default(),
            spectating: SpectatingPresenceConfig::default(),
            post_match: PartyPresenceConfig::default(),
        }
    }
}

fn validate_prefix(label: &str, prefix: &str) -> Result<(), String> {
    if prefix.chars().count() > 64 {
        return Err(format!("{label} cannot exceed 64 characters"));
    }

    if prefix
        .chars()
        .any(|character| matches!(character, '\0' | '\r' | '\n'))
    {
        return Err(format!("{label} cannot contain line breaks or NUL"));
    }

    Ok(())
}

impl DiscordPresenceConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_prefix("Discord global preview prefix", &self.global.preview_prefix)?;
        validate_prefix("Discord Hideout state prefix", &self.hideout.state_prefix)?;
        validate_prefix("Discord Sandbox state prefix", &self.sandbox.state_prefix)?;
        validate_prefix("Discord district prefix", &self.explore_nyc.district_prefix)?;
        validate_prefix(
            "Discord Main Menu state prefix",
            &self.main_menu.state_prefix,
        )?;
        validate_prefix(
            "Discord Matchmaking state prefix",
            &self.matchmaking.state_prefix,
        )?;
        validate_prefix("Discord Match state prefix", &self.r#match.state_prefix)?;
        validate_prefix(
            "Discord Spectating match ID prefix",
            &self.spectating.match_id_prefix,
        )?;
        validate_prefix(
            "Discord Post Match state prefix",
            &self.post_match.state_prefix,
        )?;

        Ok(())
    }

    pub fn reset_section(&mut self, section: DiscordPresenceSection) {
        let defaults = Self::default();
        match section {
            DiscordPresenceSection::Global => self.global = defaults.global,
            DiscordPresenceSection::Hideout => self.hideout = defaults.hideout,
            DiscordPresenceSection::Sandbox => self.sandbox = defaults.sandbox,
            DiscordPresenceSection::ExploreNyc => self.explore_nyc = defaults.explore_nyc,
            DiscordPresenceSection::Loading => self.loading = defaults.loading,
            DiscordPresenceSection::MainMenu => self.main_menu = defaults.main_menu,
            DiscordPresenceSection::Matchmaking => self.matchmaking = defaults.matchmaking,
            DiscordPresenceSection::Match => self.r#match = defaults.r#match,
            DiscordPresenceSection::Spectating => self.spectating = defaults.spectating,
            DiscordPresenceSection::PostMatch => self.post_match = defaults.post_match,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiscordPresenceSection {
    Global,
    Hideout,
    Sandbox,
    ExploreNyc,
    Loading,
    MainMenu,
    Matchmaking,
    Match,
    Spectating,
    PostMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GlobalPresenceConfig {
    pub enabled: bool,
    pub show_elapsed_time: bool,
    pub preview_prefix: String,
}

impl Default for GlobalPresenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_elapsed_time: true,
            preview_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutPresenceConfig {
    pub use_official_hero_phrase: bool,
    pub show_hero_image: bool,
    pub party_display: PartyDisplay,
    pub state_prefix: String,
}

impl Default for HideoutPresenceConfig {
    fn default() -> Self {
        Self {
            use_official_hero_phrase: true,
            show_hero_image: true,
            party_display: PartyDisplay::Compact,
            state_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SandboxPresenceConfig {
    pub show_hero_image: bool,
    pub party_display: PartyDisplay,
    pub state_prefix: String,
}

impl Default for SandboxPresenceConfig {
    fn default() -> Self {
        Self {
            show_hero_image: true,
            party_display: PartyDisplay::Compact,
            state_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreNycPresenceConfig {
    pub show_hero_in_details: bool,
    pub show_district: bool,
    pub district_prefix: String,
    pub show_district_image: bool,
    pub party_display: PartyDisplay,
}

impl Default for ExploreNycPresenceConfig {
    fn default() -> Self {
        Self {
            show_hero_in_details: true,
            show_district: true,
            district_prefix: "› ".to_string(),
            show_district_image: true,
            party_display: PartyDisplay::Compact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LoadingPresenceConfig {
    pub show_loading_state: bool,
    pub use_deadlock_logo: bool,
}

impl Default for LoadingPresenceConfig {
    fn default() -> Self {
        Self {
            show_loading_state: true,
            use_deadlock_logo: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainMenuPresenceConfig {
    pub use_deadlock_logo: bool,
    pub party_display: PartyDisplay,
    pub state_prefix: String,
}

impl Default for MainMenuPresenceConfig {
    fn default() -> Self {
        Self {
            use_deadlock_logo: true,
            party_display: PartyDisplay::Compact,
            state_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyPresenceConfig {
    pub party_display: PartyDisplay,
    pub state_prefix: String,
}

impl Default for PartyPresenceConfig {
    fn default() -> Self {
        Self {
            party_display: PartyDisplay::Compact,
            state_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchPresenceConfig {
    pub show_hero_image: bool,
    pub party_display: PartyDisplay,
    pub state_prefix: String,
}

impl Default for MatchPresenceConfig {
    fn default() -> Self {
        Self {
            show_hero_image: true,
            party_display: PartyDisplay::Compact,
            state_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PartyDisplay {
    Compact,
    Discord,
    Hidden,
}

impl Default for PartyDisplay {
    fn default() -> Self {
        Self::Compact
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MatchPresenceConfigInput {
    show_hero_image: bool,
    party_display: Option<PartyDisplay>,
    show_party: Option<bool>,
    state_prefix: String,
}

impl Default for MatchPresenceConfigInput {
    fn default() -> Self {
        Self {
            show_hero_image: true,
            party_display: None,
            show_party: None,
            state_prefix: String::new(),
        }
    }
}

impl<'de> Deserialize<'de> for MatchPresenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = MatchPresenceConfigInput::deserialize(deserializer)?;
        let party_display = input
            .party_display
            .or_else(|| {
                input.show_party.map(|show| {
                    if show {
                        PartyDisplay::Compact
                    } else {
                        PartyDisplay::Hidden
                    }
                })
            })
            .unwrap_or_default();

        Ok(Self {
            show_hero_image: input.show_hero_image,
            party_display,
            state_prefix: input.state_prefix,
        })
    }
}

fn migrated_party_display(
    party_display: Option<PartyDisplay>,
    show_party: Option<bool>,
    default: PartyDisplay,
) -> PartyDisplay {
    party_display
        .or_else(|| {
            show_party.map(|show| {
                if show {
                    PartyDisplay::Compact
                } else {
                    PartyDisplay::Hidden
                }
            })
        })
        .unwrap_or(default)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct HideoutPresenceConfigInput {
    use_official_hero_phrase: bool,
    show_hero_image: bool,
    party_display: Option<PartyDisplay>,
    show_party: Option<bool>,
    state_prefix: String,
}

impl Default for HideoutPresenceConfigInput {
    fn default() -> Self {
        let defaults = HideoutPresenceConfig::default();
        Self {
            use_official_hero_phrase: defaults.use_official_hero_phrase,
            show_hero_image: defaults.show_hero_image,
            party_display: None,
            show_party: None,
            state_prefix: defaults.state_prefix,
        }
    }
}

impl<'de> Deserialize<'de> for HideoutPresenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = HideoutPresenceConfigInput::deserialize(deserializer)?;
        Ok(Self {
            use_official_hero_phrase: input.use_official_hero_phrase,
            show_hero_image: input.show_hero_image,
            party_display: migrated_party_display(
                input.party_display,
                input.show_party,
                Self::default().party_display,
            ),
            state_prefix: input.state_prefix,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ExploreNycPresenceConfigInput {
    show_hero_in_details: bool,
    show_district: bool,
    district_prefix: String,
    show_district_image: bool,
    party_display: Option<PartyDisplay>,
    show_party: Option<bool>,
}

impl Default for ExploreNycPresenceConfigInput {
    fn default() -> Self {
        let defaults = ExploreNycPresenceConfig::default();
        Self {
            show_hero_in_details: defaults.show_hero_in_details,
            show_district: defaults.show_district,
            district_prefix: defaults.district_prefix,
            show_district_image: defaults.show_district_image,
            party_display: None,
            show_party: None,
        }
    }
}

impl<'de> Deserialize<'de> for ExploreNycPresenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = ExploreNycPresenceConfigInput::deserialize(deserializer)?;
        Ok(Self {
            show_hero_in_details: input.show_hero_in_details,
            show_district: input.show_district,
            district_prefix: input.district_prefix,
            show_district_image: input.show_district_image,
            party_display: migrated_party_display(
                input.party_display,
                input.show_party,
                Self::default().party_display,
            ),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MainMenuPresenceConfigInput {
    use_deadlock_logo: bool,
    party_display: Option<PartyDisplay>,
    show_party: Option<bool>,
    state_prefix: String,
}

impl Default for MainMenuPresenceConfigInput {
    fn default() -> Self {
        Self {
            use_deadlock_logo: true,
            party_display: None,
            show_party: None,
            state_prefix: String::new(),
        }
    }
}

impl<'de> Deserialize<'de> for MainMenuPresenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = MainMenuPresenceConfigInput::deserialize(deserializer)?;
        Ok(Self {
            use_deadlock_logo: input.use_deadlock_logo,
            party_display: migrated_party_display(
                input.party_display,
                input.show_party,
                Self::default().party_display,
            ),
            state_prefix: input.state_prefix,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PartyPresenceConfigInput {
    party_display: Option<PartyDisplay>,
    show_party: Option<bool>,
    state_prefix: String,
}

impl Default for PartyPresenceConfigInput {
    fn default() -> Self {
        Self {
            party_display: None,
            show_party: None,
            state_prefix: String::new(),
        }
    }
}

impl<'de> Deserialize<'de> for PartyPresenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = PartyPresenceConfigInput::deserialize(deserializer)?;
        Ok(Self {
            party_display: migrated_party_display(
                input.party_display,
                input.show_party,
                Self::default().party_display,
            ),
            state_prefix: input.state_prefix,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectatingPresenceConfig {
    pub show_match_id: bool,
    pub match_id_prefix: String,
}

impl Default for SpectatingPresenceConfig {
    fn default() -> Self {
        Self {
            show_match_id: true,
            match_id_prefix: String::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SpectatingPresenceConfigInput {
    show_match_id: bool,
    match_id_prefix: String,
}

impl Default for SpectatingPresenceConfigInput {
    fn default() -> Self {
        Self {
            show_match_id: true,
            match_id_prefix: String::new(),
        }
    }
}

impl<'de> Deserialize<'de> for SpectatingPresenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = SpectatingPresenceConfigInput::deserialize(deserializer)?;
        Ok(Self {
            show_match_id: input.show_match_id,
            match_id_prefix: input.match_id_prefix,
        })
    }
}

static CURRENT_CONFIG: LazyLock<RwLock<DiscordPresenceConfig>> =
    LazyLock::new(|| RwLock::new(DiscordPresenceConfig::default()));

pub(crate) fn current() -> DiscordPresenceConfig {
    CURRENT_CONFIG
        .read()
        .map(|config| config.clone())
        .unwrap_or_default()
}

pub(crate) fn apply(config: DiscordPresenceConfig) -> Result<(), String> {
    config.validate()?;
    *CURRENT_CONFIG
        .write()
        .map_err(|_| "Discord Presence configuration lock poisoned".to_string())? = config;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_current_presence_behavior() {
        let config = DiscordPresenceConfig::default();

        assert_eq!(
            config.global,
            GlobalPresenceConfig {
                enabled: true,
                show_elapsed_time: true,
                preview_prefix: String::new(),
            }
        );
        assert_eq!(
            config.hideout,
            HideoutPresenceConfig {
                use_official_hero_phrase: true,
                show_hero_image: true,
                party_display: PartyDisplay::Compact,
                state_prefix: String::new(),
            }
        );
        assert_eq!(
            config.sandbox,
            SandboxPresenceConfig {
                show_hero_image: true,
                party_display: PartyDisplay::Compact,
                state_prefix: String::new(),
            }
        );
        assert_eq!(
            config.explore_nyc,
            ExploreNycPresenceConfig {
                show_hero_in_details: true,
                show_district: true,
                district_prefix: "› ".to_string(),
                show_district_image: true,
                party_display: PartyDisplay::Compact,
            }
        );
        assert_eq!(
            config.loading,
            LoadingPresenceConfig {
                show_loading_state: true,
                use_deadlock_logo: true,
            }
        );
        assert_eq!(
            config.main_menu,
            MainMenuPresenceConfig {
                use_deadlock_logo: true,
                party_display: PartyDisplay::Compact,
                state_prefix: String::new(),
            }
        );
        assert_eq!(config.matchmaking.party_display, PartyDisplay::Compact);
        assert!(config.matchmaking.state_prefix.is_empty());
        assert_eq!(
            config.r#match,
            MatchPresenceConfig {
                show_hero_image: true,
                party_display: PartyDisplay::Compact,
                state_prefix: String::new(),
            }
        );
        assert_eq!(
            config.spectating,
            SpectatingPresenceConfig {
                show_match_id: true,
                match_id_prefix: String::new(),
            }
        );
        assert!(config.spectating.show_match_id);
        assert_eq!(config.post_match.party_display, PartyDisplay::Compact);
        assert!(config.post_match.state_prefix.is_empty());

        let serialized = serde_json::to_value(config).unwrap();
        assert!(serialized.get("match").is_some());
        assert!(serialized.get("r#match").is_none());
        for section in [
            "hideout",
            "sandbox",
            "exploreNyc",
            "mainMenu",
            "matchmaking",
            "match",
            "postMatch",
        ] {
            assert!(serialized[section].get("partyDisplay").is_some());
            assert!(serialized[section].get("showParty").is_none());
        }

        assert!(serialized["spectating"].get("partyDisplay").is_none());
        assert!(serialized["spectating"].get("showParty").is_none());
    }

    #[test]
    fn partial_config_uses_current_defaults_for_missing_fields() {
        let config: DiscordPresenceConfig = serde_json::from_str(
            r#"{
                "exploreNyc": { "showDistrict": false },
                "hideout": { "showParty": false },
                "match": { "showHeroImage": false }
            }"#,
        )
        .unwrap();

        assert!(!config.explore_nyc.show_district);
        assert!(config.explore_nyc.show_hero_in_details);
        assert_eq!(config.explore_nyc.district_prefix, "› ");
        assert_eq!(config.hideout.party_display, PartyDisplay::Hidden);
        assert!(config.hideout.use_official_hero_phrase);
        assert_eq!(config.loading, LoadingPresenceConfig::default());
        assert!(!config.r#match.show_hero_image);
        assert_eq!(config.r#match.party_display, PartyDisplay::Compact);
        assert_eq!(config.sandbox, SandboxPresenceConfig::default());
        assert_eq!(config.explore_nyc.party_display, PartyDisplay::Compact);
        assert_eq!(config.main_menu.party_display, PartyDisplay::Compact);
        assert_eq!(config.matchmaking.party_display, PartyDisplay::Compact);
        assert_eq!(config.post_match.party_display, PartyDisplay::Compact);
        assert!(config.global.preview_prefix.is_empty());
        assert!(config.hideout.state_prefix.is_empty());
        assert!(config.sandbox.state_prefix.is_empty());
        assert!(config.main_menu.state_prefix.is_empty());
        assert!(config.matchmaking.state_prefix.is_empty());
        assert!(config.r#match.state_prefix.is_empty());
        assert!(config.spectating.match_id_prefix.is_empty());
        assert!(config.post_match.state_prefix.is_empty());
    }

    #[test]
    fn legacy_match_party_boolean_migrates_to_explicit_display_mode() {
        for (legacy, expected) in [(true, PartyDisplay::Compact), (false, PartyDisplay::Hidden)] {
            let config: DiscordPresenceConfig = serde_json::from_value(serde_json::json!({
                "match": {
                    "showHeroImage": true,
                    "showParty": legacy
                }
            }))
            .unwrap();

            assert_eq!(config.r#match.party_display, expected);
            let serialized = serde_json::to_value(config).unwrap();
            assert!(serialized["match"].get("showParty").is_none());
            assert_eq!(
                serialized["match"]["partyDisplay"],
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    #[test]
    fn explicit_match_party_display_wins_over_legacy_field() {
        let config: DiscordPresenceConfig = serde_json::from_str(
            r#"{
                "match": {
                    "partyDisplay": "compact",
                    "showParty": true
                }
            }"#,
        )
        .unwrap();

        assert_eq!(config.r#match.party_display, PartyDisplay::Compact);
    }

    #[test]
    fn legacy_party_booleans_migrate_for_every_non_match_section() {
        for legacy in [true, false] {
            let config: DiscordPresenceConfig = serde_json::from_value(serde_json::json!({
                "hideout": { "showParty": legacy },
                "exploreNyc": { "showParty": legacy },
                "mainMenu": { "showParty": legacy },
                "matchmaking": { "showParty": legacy },
                "postMatch": { "showParty": legacy }
            }))
            .unwrap();
            let expected = if legacy {
                PartyDisplay::Compact
            } else {
                PartyDisplay::Hidden
            };

            assert_eq!(config.hideout.party_display, expected);
            assert_eq!(config.explore_nyc.party_display, expected);
            assert_eq!(config.main_menu.party_display, expected);
            assert_eq!(config.matchmaking.party_display, expected);
            assert_eq!(config.post_match.party_display, expected);

            let serialized = serde_json::to_value(config).unwrap();
            for section in [
                "hideout",
                "exploreNyc",
                "mainMenu",
                "matchmaking",
                "postMatch",
            ] {
                assert!(serialized["spectating"].get("partyDisplay").is_none());
                assert!(serialized["spectating"].get("showParty").is_none());
                assert!(serialized[section].get("showParty").is_none());
                assert_eq!(
                    serialized[section]["partyDisplay"],
                    serde_json::json!(if legacy { "compact" } else { "hidden" })
                );
            }
        }
    }

    #[test]
    fn explicit_party_display_wins_over_legacy_boolean_for_every_section() {
        let config: DiscordPresenceConfig = serde_json::from_value(serde_json::json!({
            "hideout": { "partyDisplay": "compact", "showParty": false },
            "exploreNyc": { "partyDisplay": "discord", "showParty": false },
            "mainMenu": { "partyDisplay": "hidden", "showParty": true },
            "matchmaking": { "partyDisplay": "compact", "showParty": false },
            "postMatch": { "partyDisplay": "compact", "showParty": false }
        }))
        .unwrap();

        assert_eq!(config.hideout.party_display, PartyDisplay::Compact);
        assert_eq!(config.explore_nyc.party_display, PartyDisplay::Discord);
        assert_eq!(config.main_menu.party_display, PartyDisplay::Hidden);
        assert_eq!(config.matchmaking.party_display, PartyDisplay::Compact);
        assert_eq!(config.post_match.party_display, PartyDisplay::Compact);
    }

    #[test]
    fn spectating_match_id_setting_round_trips_independently() {
        let config: DiscordPresenceConfig = serde_json::from_value(serde_json::json!({
            "spectating": {
                "showMatchId": false,
                "matchIdPrefix": "› ",
                "partyDisplay": "compact",
                "showParty": true
            }
        }))
        .unwrap();

        assert!(!config.spectating.show_match_id);
        assert_eq!(config.spectating.match_id_prefix, "› ");

        let serialized = serde_json::to_value(config).unwrap();

        assert_eq!(serialized["spectating"]["showMatchId"], false);
        assert_eq!(serialized["spectating"]["matchIdPrefix"], "› ");
        assert!(serialized["spectating"].get("partyDisplay").is_none());
        assert!(serialized["spectating"].get("showParty").is_none());
    }

    #[test]
    fn resetting_one_section_or_everything_restores_defaults() {
        let mut config = DiscordPresenceConfig::default();
        config.global.preview_prefix = "G ".to_string();
        config.explore_nyc.district_prefix = "• ".to_string();
        config.hideout.party_display = PartyDisplay::Hidden;
        config.hideout.state_prefix = "H ".to_string();
        config.sandbox.party_display = PartyDisplay::Hidden;
        config.sandbox.show_hero_image = false;
        config.sandbox.state_prefix = "S ".to_string();
        config.main_menu.state_prefix = "N ".to_string();
        config.matchmaking.state_prefix = "Q ".to_string();
        config.spectating.show_match_id = false;
        config.spectating.match_id_prefix = "S ".to_string();
        config.r#match.party_display = PartyDisplay::Hidden;
        config.r#match.state_prefix = "M ".to_string();
        config.post_match.state_prefix = "P ".to_string();

        config.reset_section(DiscordPresenceSection::Global);
        assert_eq!(config.global, GlobalPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::Hideout);
        assert_eq!(config.hideout, HideoutPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::Sandbox);
        assert_eq!(config.sandbox, SandboxPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::ExploreNyc);
        assert_eq!(
            config.explore_nyc,
            DiscordPresenceConfig::default().explore_nyc
        );
        config.reset_section(DiscordPresenceSection::MainMenu);
        assert_eq!(config.main_menu, MainMenuPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::Matchmaking);
        assert_eq!(config.matchmaking, PartyPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::Spectating);
        assert_eq!(config.spectating, SpectatingPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::Match);
        assert_eq!(config.r#match, MatchPresenceConfig::default());

        config.reset_section(DiscordPresenceSection::PostMatch);
        assert_eq!(config.post_match, PartyPresenceConfig::default());

        assert_eq!(config, DiscordPresenceConfig::default());
    }

    #[test]
    fn every_party_section_reset_restores_its_default_mode() {
        for section in [
            DiscordPresenceSection::Hideout,
            DiscordPresenceSection::Sandbox,
            DiscordPresenceSection::ExploreNyc,
            DiscordPresenceSection::MainMenu,
            DiscordPresenceSection::Matchmaking,
            DiscordPresenceSection::Match,
            DiscordPresenceSection::PostMatch,
        ] {
            let mut config = DiscordPresenceConfig::default();
            config.hideout.party_display = PartyDisplay::Hidden;
            config.sandbox.party_display = PartyDisplay::Hidden;
            config.explore_nyc.party_display = PartyDisplay::Hidden;
            config.main_menu.party_display = PartyDisplay::Hidden;
            config.matchmaking.party_display = PartyDisplay::Hidden;
            config.r#match.party_display = PartyDisplay::Hidden;
            config.post_match.party_display = PartyDisplay::Hidden;

            config.reset_section(section.clone());
            let defaults = DiscordPresenceConfig::default();
            match section {
                DiscordPresenceSection::Hideout => assert_eq!(config.hideout, defaults.hideout),
                DiscordPresenceSection::Sandbox => assert_eq!(config.sandbox, defaults.sandbox),
                DiscordPresenceSection::ExploreNyc => {
                    assert_eq!(config.explore_nyc, defaults.explore_nyc)
                }
                DiscordPresenceSection::MainMenu => {
                    assert_eq!(config.main_menu, defaults.main_menu)
                }
                DiscordPresenceSection::Matchmaking => {
                    assert_eq!(config.matchmaking, defaults.matchmaking)
                }
                DiscordPresenceSection::Match => assert_eq!(config.r#match, defaults.r#match),
                DiscordPresenceSection::PostMatch => {
                    assert_eq!(config.post_match, defaults.post_match)
                }
                DiscordPresenceSection::Global
                | DiscordPresenceSection::Loading
                | DiscordPresenceSection::Spectating => unreachable!(),
            }
        }
    }

    #[test]
    fn all_presence_prefixes_validate_and_round_trip() {
        let mut config = DiscordPresenceConfig::default();
        config.global.preview_prefix = "★ ".to_string();
        config.hideout.state_prefix = "› ".to_string();
        config.sandbox.state_prefix = "Sandbox: ".to_string();
        config.explore_nyc.district_prefix = "• ".to_string();
        config.main_menu.state_prefix = "· ".to_string();
        config.matchmaking.state_prefix = "— ".to_string();
        config.r#match.state_prefix = ">> ".to_string();
        config.spectating.match_id_prefix = "# ".to_string();
        config.post_match.state_prefix = "✓ ".to_string();

        assert!(config.validate().is_ok());
        let serialized = serde_json::to_string(&config).unwrap();
        let restored: DiscordPresenceConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored, config);
    }

    #[test]
    fn presence_prefixes_reject_more_than_sixty_four_characters() {
        let mut config = DiscordPresenceConfig::default();
        config.sandbox.state_prefix = "x".repeat(65);

        assert!(config.validate().is_err());
    }

    #[test]
    fn presence_prefixes_reject_line_breaks_and_nul() {
        for invalid in ["hello\n", "hello\r", "hello\0"] {
            let mut config = DiscordPresenceConfig::default();
            config.spectating.match_id_prefix = invalid.to_string();

            assert!(config.validate().is_err());
        }
    }
}
