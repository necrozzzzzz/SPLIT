import { useEffect, useState } from "react";
import { isBorderlessEdition } from "./edition";
import "./onboarding.css";

const KEY = "split.onboarding.v2.completed";

const steps = [
  { title: "Welcome to SPLIT", eyebrow: "GET STARTED", body: "SPLIT lets you save and restore positions in Deadlock, organize your practice routes, and show your current activity on Discord.", note: "SPLIT is a practice companion. Position tools are not intended for live matches." },
  { title: "Save and load positions", eyebrow: "SAVE STATES", body: "You have eight slots in each preset. Save your current position to a slot, then load it to return there. Give slots names and colors so you can find them quickly.", note: "Saved positions can include a screenshot for easy identification." },
  { title: "Presets and favorites", eyebrow: "ORGANIZE", body: "Use four presets to keep different routes or practice sessions separate. Favorite Mode gives you a separate set of slots for positions you return to often.", note: "You can rename presets, import and export presets, and use Undo / Redo to reverse supported edits." },
  { title: "Shortcuts and Quick Access", eyebrow: "PLAY FASTER", body: "Configure your save, load, preset, favorites, Undo and Redo shortcuts in Settings > Hotkeys.", note: isBorderlessEdition ? "Borderless also includes Quick Access, an in-game overlay for working with slots, and configurable in-game notifications." : "Fullscreen keeps keyboard-driven save states and Discord Presence. Quick Access and in-game notification overlays are available in the Borderless edition." },
  { title: "Discord Rich Presence", eyebrow: "SHOW YOUR ACTIVITY", body: "SPLIT can display your Deadlock activity on Discord, including the Hideout, Explore NYC, matchmaking and matches. Hero portraits and context details update automatically when detected.", note: "Customize what appears in Settings > Discord. Discord must be running for others to see your activity." },
  { title: "Make SPLIT yours", eyebrow: "SETTINGS", body: "Explore general settings, startup sound, shortcuts, Discord Presence and other options. You can revisit this guide from Settings > About at any time.", note: "You're ready to start practicing!" },
];

export function isOnboardingCompleted(): boolean {
  try { return window.localStorage.getItem(KEY) === "true"; }
  catch { return false; }
}

export default function Onboarding({ onClose }: { onClose: () => void }) {
  const [index, setIndex] = useState(0);
  const step = steps[index];
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);
  const finish = () => {
    try { window.localStorage.setItem(KEY, "true"); } catch { /* storage unavailable */ }
    onClose();
  };
  return (
    <div className="onboarding-backdrop">
      <section className="onboarding-panel" role="dialog" aria-modal="true" aria-labelledby="onboarding-title">
        <div className="onboarding-top">
          <span className="onboarding-brand">SPLIT <span>2.0</span></span>
          <button type="button" className="onboarding-skip" onClick={finish}>Skip guide</button>
        </div>
        <div className="onboarding-progress" aria-label={`Step ${index + 1} of ${steps.length}`}>
          {steps.map((_, i) => <span key={i} className={i <= index ? "active" : ""} />)}
        </div>
        <div className="onboarding-content" key={index}>
          <p className="onboarding-eyebrow">{step.eyebrow}</p>
          <h2 id="onboarding-title">{step.title}</h2>
          <p className="onboarding-body">{step.body}</p>
          <p className="onboarding-note">{step.note}</p>
        </div>
        <footer className="onboarding-footer">
          <span>{index + 1} / {steps.length}</span>
          <div className="onboarding-actions">
            {index > 0 && <button type="button" className="onboarding-secondary" onClick={() => setIndex(index - 1)}>Back</button>}
            <button type="button" className="onboarding-primary" onClick={() => index === steps.length - 1 ? finish() : setIndex(index + 1)}>{index === steps.length - 1 ? "Get started" : "Next"}</button>
          </div>
        </footer>
      </section>
    </div>
  );
}
