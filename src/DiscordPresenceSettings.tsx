import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import abramsUrl from "../assets/discord-preview/abrams.png?url";
import bebopUrl from "../assets/discord-preview/bebop.png?url";
import calicoUrl from "../assets/discord-preview/calico.png?url";
import deadlockLogoUrl from "../assets/discord-preview/deadlock_logo.png?url";
import dynamoUrl from "../assets/discord-preview/dynamo.png?url";
import exploreNycDefaultUrl from "../assets/discord-preview/explore_nyc_default.png?url";
import hazeUrl from "../assets/discord-preview/haze.png?url";
import infernusUrl from "../assets/discord-preview/infernus.png?url";
import ivyUrl from "../assets/discord-preview/ivy.png?url";
import lashUrl from "../assets/discord-preview/lash.png?url";
import mcginnisUrl from "../assets/discord-preview/mcginnis.png?url";
import mirageUrl from "../assets/discord-preview/mirage.png?url";
import paradoxUrl from "../assets/discord-preview/paradox.png?url";
import pocketUrl from "../assets/discord-preview/pocket.png?url";
import ratKingUrl from "../assets/discord-preview/rat_king.png?url";
import shivUrl from "../assets/discord-preview/shiv.png?url";
import venatorUrl from "../assets/discord-preview/venator.png?url";
import vindictaUrl from "../assets/discord-preview/vindicta.png?url";
import viscousUrl from "../assets/discord-preview/viscous.png?url";
import wraithUrl from "../assets/discord-preview/wraith.png?url";
import yamatoUrl from "../assets/discord-preview/yamato.png?url";

type PartyDisplay = "compact" | "discord" | "hidden";

type DiscordPresenceConfig = {
  global: { enabled: boolean; showElapsedTime: boolean; previewPrefix: string };
  hideout: {
    useOfficialHeroPhrase: boolean;
    showHeroImage: boolean;
    partyDisplay: PartyDisplay;
    statePrefix: string;
  };
  sandbox: {
    showHeroImage: boolean;
    partyDisplay: PartyDisplay;
    statePrefix: string;
  };
  exploreNyc: {
    showHeroInDetails: boolean;
    showDistrict: boolean;
    districtPrefix: string;
    showDistrictImage: boolean;
    partyDisplay: PartyDisplay;
  };
  loading: { showLoadingState: boolean; useDeadlockLogo: boolean };
  mainMenu: { useDeadlockLogo: boolean; partyDisplay: PartyDisplay; statePrefix: string };
  matchmaking: { partyDisplay: PartyDisplay; statePrefix: string };
  match: { showHeroImage: boolean; partyDisplay: PartyDisplay; statePrefix: string };
  spectating: { showMatchId: boolean; matchIdPrefix: string };
  postMatch: { partyDisplay: PartyDisplay; statePrefix: string };
};

type PresenceSection = keyof DiscordPresenceConfig;

const SECTIONS: Array<{ key: PresenceSection; label: string }> = [
  { key: "global", label: "Global" },
  { key: "hideout", label: "Hideout" },
  { key: "sandbox", label: "Sandbox" },
  { key: "exploreNyc", label: "Explore NYC" },
  { key: "loading", label: "Loading" },
  { key: "mainMenu", label: "Main Menu" },
  { key: "matchmaking", label: "Matchmaking" },
  { key: "match", label: "Match" },
  { key: "spectating", label: "Spectating" },
  { key: "postMatch", label: "Post Match" },
];

const PREFIX_PRESETS = [
  { label: "None", value: "" },
  { label: "›", value: "› " },
  { label: "•", value: "• " },
  { label: "·", value: "· " },
  { label: "—", value: "— " },
] as const;

type PreviewHero = {
  name: string;
  imageUrl: string;
  hideoutPhrase: string;
};

const PREVIEW_HEROES: readonly PreviewHero[] = [
  {
    name: "Abrams",
    imageUrl: abramsUrl,
    hideoutPhrase: "Investigating the Hideout",
  },
  {
    name: "Bebop",
    imageUrl: bebopUrl,
    hideoutPhrase: "Ignoring Lash in the Hideout",
  },
  {
    name: "Calico",
    imageUrl: calicoUrl,
    hideoutPhrase: "Playing With Ava in the Hideout",
  },
  {
    name: "Dynamo",
    imageUrl: dynamoUrl,
    hideoutPhrase: "Pontificating in the Hideout",
  },
  {
    name: "Haze",
    imageUrl: hazeUrl,
    hideoutPhrase: "Sleep Walking in the Hideout",
  },
  {
    name: "Infernus",
    imageUrl: infernusUrl,
    hideoutPhrase: "Mixing Drinks in the Hideout",
  },
  {
    name: "Ivy",
    imageUrl: ivyUrl,
    hideoutPhrase: "Wishing the Arroyos were in the Hideout",
  },
  {
    name: "Lash",
    imageUrl: lashUrl,
    hideoutPhrase: "Thinking About Lash in the Hideout",
  },
  {
    name: "McGinnis",
    imageUrl: mcginnisUrl,
    hideoutPhrase: "Tinkering in the Hideout",
  },
  {
    name: "Mirage",
    imageUrl: mirageUrl,
    hideoutPhrase: "Dreaming of Wyoming in the Hideout",
  },
  {
    name: "Paradox",
    imageUrl: paradoxUrl,
    hideoutPhrase: "Scheming in the Hideout",
  },
  {
    name: "Pocket",
    imageUrl: pocketUrl,
    hideoutPhrase: "Sulking in the Hideout",
  },
  {
    name: "Rat King",
    imageUrl: ratKingUrl,
    hideoutPhrase: "Wishing the Hideout was on Long Island",
  },
  {
    name: "Shiv",
    imageUrl: shivUrl,
    hideoutPhrase: "Playing With Knives in the Hideout",
  },
  {
    name: "Venator",
    imageUrl: venatorUrl,
    hideoutPhrase: "Blessing Ammunition in the Hideout",
  },
  {
    name: "Vindicta",
    imageUrl: vindictaUrl,
    hideoutPhrase: "Brooding in the Hideout",
  },
  {
    name: "Viscous",
    imageUrl: viscousUrl,
    hideoutPhrase: "Wishing the Hideout was The Cube",
  },
  {
    name: "Wraith",
    imageUrl: wraithUrl,
    hideoutPhrase: "Gambling in the Hideout",
  },
  {
    name: "Yamato",
    imageUrl: yamatoUrl,
    hideoutPhrase: "Reminiscing in the Hideout",
  },
];

export function chooseRandomPreviewHero(
  random: () => number = Math.random,
): PreviewHero {
  const index = Math.min(
    PREVIEW_HEROES.length - 1,
    Math.floor(random() * PREVIEW_HEROES.length),
  );
  return PREVIEW_HEROES[index];
}

// Module initialization runs once per WebView process, so remounts keep one hero.
export const SESSION_PREVIEW_HERO = chooseRandomPreviewHero();

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function ToggleRow({
  checked,
  description,
  disabled = false,
  label,
  onChange,
}: {
  checked: boolean;
  description?: string;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className={`drp-option ${disabled ? "disabled" : ""}`}>
      <span>
        <strong>{label}</strong>
        {description ? <small>{description}</small> : null}
      </span>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span className="drp-switch" aria-hidden="true" />
    </label>
  );
}

function PartyDisplayRow({
  helper,
  onChange,
  value,
}: {
  helper?: string;
  onChange: (value: PartyDisplay) => void;
  value: PartyDisplay;
}) {
  return (
    <label className="drp-select-option">
      <span>
        <strong>Party display</strong>
        <small>
          {value === "compact"
            ? "Shows 1/6 inline"
            : value === "discord"
              ? "Uses Discord's native party display"
              : "Hides party size"}
        </small>
        {helper ? <small>{helper}</small> : null}
      </span>
      <select
        value={value}
        onChange={(event) => onChange(event.currentTarget.value as PartyDisplay)}
      >
        <option value="compact">Compact</option>
        <option value="discord">Discord</option>
        <option value="hidden">Hidden</option>
      </select>
    </label>
  );
}

function compactState(state: string, display: PartyDisplay): string {
  return display === "compact" ? `${state} \u00b7 1/6` : state;
}

function PrefixSelector({
  disabled = false,
  label,
  onChange,
  onPreviewChange,
  value,
}: {
  disabled?: boolean;
  label: string;
  onChange: (value: string) => void;
  onPreviewChange: (value: string | null) => void;
  value: string;
}) {
  const [customSelected, setCustomSelected] = useState(
    !PREFIX_PRESETS.some((preset) => preset.value === value),
  );
  const [draft, setDraft] = useState(value);

  useEffect(() => {
    setDraft(value);
    setCustomSelected(!PREFIX_PRESETS.some((preset) => preset.value === value));
  }, [value]);

  return (
    <fieldset className="drp-prefix-fieldset" disabled={disabled}>
      <legend>{label}</legend>
      <div className="drp-prefix-presets">
        {PREFIX_PRESETS.map((preset) => (
          <button
            key={preset.label}
            type="button"
            className={!customSelected && value === preset.value ? "active" : ""}
            onClick={() => {
              setCustomSelected(false);
              setDraft(preset.value);
              onPreviewChange(null);
              onChange(preset.value);
            }}
          >
            {preset.label}
          </button>
        ))}
        <button
          type="button"
          className={customSelected ? "active" : ""}
          onClick={() => {
            setCustomSelected(true);
            setDraft(value);
            onPreviewChange(value);
          }}
        >
          Custom
        </button>
      </div>
      {customSelected ? (
        <label className="drp-custom-prefix">
          <span>Custom prefix</span>
          <input
            type="text"
            maxLength={64}
            value={draft}
            placeholder="Enter a short prefix"
            onChange={(event) => {
              const next = event.currentTarget.value;
              setDraft(next);
              onPreviewChange(next);
            }}
            onBlur={() => {
              if (draft !== value) onChange(draft);
              onPreviewChange(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
            }}
          />
        </label>
      ) : null}
    </fieldset>
  );
}

function previewFor(
  section: PresenceSection,
  config: DiscordPresenceConfig,
  prefixPreview: string | null,
) {
  switch (section) {
    case "hideout":
      return {
        details: config.hideout.useOfficialHeroPhrase
          ? SESSION_PREVIEW_HERO.hideoutPhrase
          : "Deadlock",
        state: compactState(
          `${prefixPreview ?? config.hideout.statePrefix}Hideout`,
          config.hideout.partyDisplay,
        ),
        imageUrl: config.hideout.showHeroImage
          ? SESSION_PREVIEW_HERO.imageUrl
          : deadlockLogoUrl,
        party: config.hideout.partyDisplay === "discord",
      };
    case "sandbox":
      return {
        details: "Sandbox",
        state: compactState(
          `${prefixPreview ?? config.sandbox.statePrefix}Practicing with ${SESSION_PREVIEW_HERO.name}`,
          config.sandbox.partyDisplay,
        ),
        imageUrl: config.sandbox.showHeroImage
          ? SESSION_PREVIEW_HERO.imageUrl
          : deadlockLogoUrl,
        party: config.sandbox.partyDisplay === "discord",
      };
    case "exploreNyc":
      return {
        details: config.exploreNyc.showHeroInDetails
          ? `Exploring NYC with ${SESSION_PREVIEW_HERO.name}`
          : "Exploring NYC",
        state: compactState(
          config.exploreNyc.showDistrict
            ? `${prefixPreview ?? config.exploreNyc.districtPrefix}Haunted Lot`
            : "Explore NYC",
          config.exploreNyc.partyDisplay,
        ),
        imageUrl: exploreNycDefaultUrl,
        party: config.exploreNyc.partyDisplay === "discord",
      };
    case "loading":
      return {
        details: "Deadlock",
        state: config.loading.showLoadingState ? "Loading..." : "Loading hidden",
        imageUrl: config.loading.useDeadlockLogo ? deadlockLogoUrl : null,
        party: false,
      };
    case "mainMenu":
      return {
        details: "Deadlock",
        state: compactState(
          `${prefixPreview ?? config.mainMenu.statePrefix}In Menu`,
          config.mainMenu.partyDisplay,
        ),
        imageUrl: config.mainMenu.useDeadlockLogo ? deadlockLogoUrl : null,
        party: config.mainMenu.partyDisplay === "discord",
      };
    case "matchmaking":
      return {
        details: "Searching for Match",
        state: compactState(
          `${prefixPreview ?? config.matchmaking.statePrefix}Matchmaking`,
          config.matchmaking.partyDisplay,
        ),
        imageUrl: deadlockLogoUrl,
        party: config.matchmaking.partyDisplay === "discord",
      };
    case "match":
      return {
        details: "In Match",
        state: compactState(
          `${prefixPreview ?? config.match.statePrefix}Playing as ${SESSION_PREVIEW_HERO.name}`,
          config.match.partyDisplay,
        ),
        imageUrl: config.match.showHeroImage
          ? SESSION_PREVIEW_HERO.imageUrl
          : deadlockLogoUrl,
        party: config.match.partyDisplay === "discord",
      };
    case "spectating":
      return {
        details: "Spectating a game",
        state: config.spectating.showMatchId
          ? `${prefixPreview ?? config.spectating.matchIdPrefix}Match 110501755`
          : "",
        imageUrl: deadlockLogoUrl,
        party: false,
      };
    case "postMatch":
      return {
        details: "Post Match",
        state: compactState(
          `${prefixPreview ?? config.postMatch.statePrefix}Deadlock`,
          config.postMatch.partyDisplay,
        ),
        imageUrl: deadlockLogoUrl,
        party: config.postMatch.partyDisplay === "discord",
      };
    default:
      return {
        details: "Deadlock",
        state: `${prefixPreview ?? config.global.previewPrefix}In Menu`,
        imageUrl: deadlockLogoUrl,
        party: false,
      };
  }
}

export default function DiscordPresenceSettings() {
  const [config, setConfig] = useState<DiscordPresenceConfig | null>(null);
  const [activeSection, setActiveSection] = useState<PresenceSection>("global");
  const [error, setError] = useState<string | null>(null);
  const [pendingSaves, setPendingSaves] = useState(0);
  const [resetting, setResetting] = useState(false);
  const [prefixPreview, setPrefixPreview] = useState<string | null>(null);
  const configRef = useRef<DiscordPresenceConfig | null>(null);
  const confirmedConfigRef = useRef<DiscordPresenceConfig | null>(null);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const latestRevisionRef = useRef(0);
  const resettingRef = useRef(false);

  useEffect(() => {
    let disposed = false;
    void invoke<DiscordPresenceConfig>("get_discord_presence_config")
      .then((loaded) => {
        if (disposed) return;
        configRef.current = loaded;
        confirmedConfigRef.current = loaded;
        setConfig(loaded);
      })
      .catch((reason) => {
        if (!disposed) {
          setError(`Could not load Discord Presence settings: ${errorMessage(reason)}`);
        }
      });
    return () => {
      disposed = true;
    };
  }, []);

  const saveConfig = useCallback((next: DiscordPresenceConfig) => {
    const revision = ++latestRevisionRef.current;
    configRef.current = next;
    setConfig(next);
    setError(null);
    setPendingSaves((count) => count + 1);

    const run = async () => {
      const saved = await invoke<DiscordPresenceConfig>(
        "update_discord_presence_config",
        { config: next },
      );
      confirmedConfigRef.current = saved;
      if (revision === latestRevisionRef.current) {
        configRef.current = saved;
        setConfig(saved);
      }
    };

    const queued = saveQueueRef.current.then(run, run);
    saveQueueRef.current = queued.then(
      () => undefined,
      () => undefined,
    );
    void queued
      .catch((reason) => {
        if (revision === latestRevisionRef.current) {
          const confirmed = confirmedConfigRef.current;
          if (confirmed) {
            configRef.current = confirmed;
            setConfig(confirmed);
          }
          setError(`Could not save Discord Presence settings: ${errorMessage(reason)}`);
        }
      })
      .finally(() => setPendingSaves((count) => Math.max(0, count - 1)));
  }, []);

  const updateSection = useCallback(
    <Section extends PresenceSection>(
      section: Section,
      patch: Partial<DiscordPresenceConfig[Section]>,
    ) => {
      const current = configRef.current;
      if (!current || resettingRef.current) return;
      saveConfig({
        ...current,
        [section]: { ...current[section], ...patch },
      });
    },
    [saveConfig],
  );

  const reset = useCallback(async (section: PresenceSection | null) => {
    if (resettingRef.current) return;
    resettingRef.current = true;
    setResetting(true);
    await saveQueueRef.current;
    setError(null);
    try {
      const saved = await invoke<DiscordPresenceConfig>(
        "reset_discord_presence_config",
        { section },
      );
      latestRevisionRef.current += 1;
      configRef.current = saved;
      confirmedConfigRef.current = saved;
      setConfig(saved);
    } catch (reason) {
      setError(`Could not reset Discord Presence settings: ${errorMessage(reason)}`);
    } finally {
      resettingRef.current = false;
      setResetting(false);
    }
  }, []);

  if (!config) {
    return (
      <section className="drp-settings-shell" aria-busy="true">
        <div className="drp-loading">
          <span className="drp-spinner" aria-hidden="true" />
          <strong>Loading Discord Presence settings...</strong>
          {error ? <p role="alert">{error}</p> : null}
        </div>
      </section>
    );
  }

  const preview = previewFor(activeSection, config, prefixPreview);
  const activeLabel =
    SECTIONS.find((section) => section.key === activeSection)?.label ?? "Global";
  const districtControlsDisabled = !config.exploreNyc.showDistrict;

  return (
    <section className="drp-settings-shell">
      <header className="drp-settings-header">
        <div>
          <p className="label">DISCORD</p>
          <h2>Discord Rich Presence</h2>
          <p>Show your current Deadlock activity on Discord.</p>
        </div>
        <div className="drp-header-actions">
          <span className="drp-save-status" role="status" aria-live="polite">
            {resetting
              ? "Resetting..."
              : pendingSaves > 0
                ? "Saving..."
                : "Changes save automatically"}
          </span>
          <button
            className="drp-reset-all"
            type="button"
            disabled={resetting}
            onClick={() => void reset(null)}
          >
            Reset all DRP settings
          </button>
        </div>
      </header>

      {error ? (
        <div className="drp-error" role="alert">
          <strong>Discord Presence settings were not updated.</strong>
          <span>{error}</span>
        </div>
      ) : null}

      <div className="drp-layout">
        <nav className="drp-section-nav" aria-label="Discord Presence contexts">
          {SECTIONS.map((section) => (
            <button
              key={section.key}
              type="button"
              className={activeSection === section.key ? "active" : ""}
              aria-current={activeSection === section.key ? "page" : undefined}
              onClick={() => {
                setPrefixPreview(null);
                setActiveSection(section.key);
              }}
            >
              {section.label}
            </button>
          ))}
        </nav>

        <div className="drp-context-grid">
          <div
            className={`drp-options-card ${resetting ? "resetting" : ""}`}
            aria-busy={resetting}
          >
            <div className="drp-context-heading">
              <div>
                <span>CONTEXT</span>
                <h3>{activeLabel}</h3>
              </div>
              <button
                type="button"
                disabled={resetting}
                onClick={() => void reset(activeSection)}
              >
                Reset section
              </button>
            </div>

            <div className="drp-options-list">
              {activeSection === "global" ? (
                <>
                  <ToggleRow
                    label="Enabled"
                    checked={config.global.enabled}
                    onChange={(enabled) => updateSection("global", { enabled })}
                  />
                  <ToggleRow
                    label="Show elapsed time"
                    checked={config.global.showElapsedTime}
                    onChange={(showElapsedTime) =>
                      updateSection("global", { showElapsedTime })
                    }
                  />
                  <PrefixSelector
                    label="State prefix"
                    value={config.global.previewPrefix}
                    onChange={(previewPrefix) => updateSection("global", { previewPrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}

              {activeSection === "hideout" ? (
                <>
                  <ToggleRow
                    label="Use official hero phrase"
                    description="Use Deadlock's official hero-specific Hideout activity text."
                    checked={config.hideout.useOfficialHeroPhrase}
                    onChange={(useOfficialHeroPhrase) =>
                      updateSection("hideout", { useOfficialHeroPhrase })
                    }
                  />
                  <ToggleRow label="Show hero image" checked={config.hideout.showHeroImage} onChange={(showHeroImage) => updateSection("hideout", { showHeroImage })} />
                  <PartyDisplayRow value={config.hideout.partyDisplay} onChange={(partyDisplay) => updateSection("hideout", { partyDisplay })} />
                  <PrefixSelector
                    label="State prefix"
                    value={config.hideout.statePrefix}
                    onChange={(statePrefix) => updateSection("hideout", { statePrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}

              {activeSection === "sandbox" ? (
                <>
                  <ToggleRow
                    label="Show hero image"
                    checked={config.sandbox.showHeroImage}
                    onChange={(showHeroImage) =>
                      updateSection("sandbox", { showHeroImage })
                    }
                  />
                  <PartyDisplayRow
                    value={config.sandbox.partyDisplay}
                    onChange={(partyDisplay) =>
                      updateSection("sandbox", { partyDisplay })
                    }
                  />
                  <PrefixSelector
                    label="Hero text prefix"
                    value={config.sandbox.statePrefix}
                    onChange={(statePrefix) => updateSection("sandbox", { statePrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}

              {activeSection === "exploreNyc" ? (
                <>
                  <ToggleRow label="Show hero in activity text" checked={config.exploreNyc.showHeroInDetails} onChange={(showHeroInDetails) => updateSection("exploreNyc", { showHeroInDetails })} />
                  <ToggleRow label="Show district" checked={config.exploreNyc.showDistrict} onChange={(showDistrict) => updateSection("exploreNyc", { showDistrict })} />
                  <ToggleRow label="Show district image" checked={config.exploreNyc.showDistrictImage} disabled={districtControlsDisabled} onChange={(showDistrictImage) => updateSection("exploreNyc", { showDistrictImage })} />
                  <PartyDisplayRow value={config.exploreNyc.partyDisplay} onChange={(partyDisplay) => updateSection("exploreNyc", { partyDisplay })} />
                  <PrefixSelector
                    label="District prefix"
                    value={config.exploreNyc.districtPrefix}
                    disabled={districtControlsDisabled}
                    onChange={(districtPrefix) => updateSection("exploreNyc", { districtPrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}

              {activeSection === "loading" ? (
                <>
                  <ToggleRow label="Show Loading state" checked={config.loading.showLoadingState} onChange={(showLoadingState) => updateSection("loading", { showLoadingState })} />
                  <ToggleRow label="Use Deadlock logo" checked={config.loading.useDeadlockLogo} onChange={(useDeadlockLogo) => updateSection("loading", { useDeadlockLogo })} />
                </>
              ) : null}

              {activeSection === "mainMenu" ? (
                <>
                  <ToggleRow label="Use Deadlock logo" checked={config.mainMenu.useDeadlockLogo} onChange={(useDeadlockLogo) => updateSection("mainMenu", { useDeadlockLogo })} />
                  <PartyDisplayRow value={config.mainMenu.partyDisplay} onChange={(partyDisplay) => updateSection("mainMenu", { partyDisplay })} />
                  <PrefixSelector
                    label="State prefix"
                    value={config.mainMenu.statePrefix}
                    onChange={(statePrefix) => updateSection("mainMenu", { statePrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}

              {activeSection === "matchmaking" ? (
                <>
                  <PartyDisplayRow value={config.matchmaking.partyDisplay} onChange={(partyDisplay) => updateSection("matchmaking", { partyDisplay })} />
                  <PrefixSelector
                    label="State prefix"
                    value={config.matchmaking.statePrefix}
                    onChange={(statePrefix) => updateSection("matchmaking", { statePrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}
              {activeSection === "match" ? (
                <>
                  <ToggleRow label="Show hero image" checked={config.match.showHeroImage} onChange={(showHeroImage) => updateSection("match", { showHeroImage })} />
                  <PartyDisplayRow value={config.match.partyDisplay} onChange={(partyDisplay) => updateSection("match", { partyDisplay })} />
                  <PrefixSelector
                    label="Hero text prefix"
                    value={config.match.statePrefix}
                    onChange={(statePrefix) => updateSection("match", { statePrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}
              {activeSection === "spectating" ? (
                <>
                  <ToggleRow label="Show match ID" checked={config.spectating.showMatchId} onChange={(showMatchId) => updateSection("spectating", { showMatchId })} />
                  <PrefixSelector
                    label="Match ID prefix"
                    value={config.spectating.matchIdPrefix}
                    onChange={(matchIdPrefix) => updateSection("spectating", { matchIdPrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}
              {activeSection === "postMatch" ? (
                <>
                  <PartyDisplayRow value={config.postMatch.partyDisplay} onChange={(partyDisplay) => updateSection("postMatch", { partyDisplay })} />
                  <PrefixSelector
                    label="State prefix"
                    value={config.postMatch.statePrefix}
                    onChange={(statePrefix) => updateSection("postMatch", { statePrefix })}
                    onPreviewChange={setPrefixPreview}
                  />
                </>
              ) : null}
            </div>
          </div>

          <aside className="drp-preview-panel" aria-label={`${activeLabel} preview`}>
            <div className="drp-preview-label">
              <span>LIVE PREVIEW</span>
              <em>Example only</em>
            </div>
            <div className={`drp-preview-card ${!config.global.enabled ? "disabled" : ""}`}>
              <strong className="drp-preview-app">Deadlock - SPLIT</strong>
              {config.global.enabled ? (
                <div
                  className={`drp-preview-activity ${
                    preview.imageUrl ? "" : "without-image"
                  }`}
                >
                  {preview.imageUrl ? (
                    <img
                      className="drp-preview-image"
                      src={preview.imageUrl}
                      alt=""
                      aria-hidden="true"
                    />
                  ) : null}
                  <div className="drp-preview-copy">
                    <strong>{preview.details}</strong>
                    {preview.state ? <span>{preview.state}</span> : null}
                    {preview.party ? <span>1 of 6</span> : null}
                    {config.global.showElapsedTime ? <span>02:14 elapsed</span> : null}
                  </div>
                </div>
              ) : <p className="drp-preview-disabled">Rich Presence is disabled.</p>}
            </div>
            <p>This preview stays inside SPLIT and never sends test activity to Discord.</p>
          </aside>
        </div>
      </div>
    </section>
  );
}
