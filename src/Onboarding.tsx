import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { isBorderlessEdition } from "./edition";
import "./onboarding.css";

const KEY = "split.onboarding.v2.completed";
type Section = "general" | "discord" | "hotkeys" | "notifications" | "diagnostics" | "about";
type Step = { title: string; description: string; target: string; view: "slots" | "settings"; section?: Section; tips?: string[] };
const steps: Step[] = [
  { title: "Welcome to SPLIT", description: "Your Deadlock practice companion. This short interactive tour will show you where to find the tools you need. Nothing will be saved or loaded during the tour.", target: "", view: "slots" },
  { title: "Your presets", description: "Start here to organize your practice positions. You have four presets, each with eight independent slots. You can rename and switch between presets.", target: ".sidebar-nav button:first-child", view: "slots" },
  { title: "Save State slots", description: "Each card represents a saved position. Save your current location, then load it to return to that spot in Deadlock. Slots can have names, colors and screenshots.", target: ".slots-grid .slot-card:first-child", view: "slots", tips: ["Right-click a saved slot’s preview to view its screenshot (when available).", "Click a slot name to rename it."] },
  { title: "Slot actions", description: "Open the three-dot menu on a slot to find its actions, including renaming, clearing, and copying a saved position when available.", target: ".slots-grid .slot-card:first-child .slot-card-menu", view: "slots", tips: ["The three-dot menu also includes View screenshot when a screenshot is available."] },
  { title: "Undo and redo", description: "Use these buttons to undo or redo supported changes to your saved positions. Their keyboard shortcuts are shown beside them.", target: ".header-history-actions", view: "slots" },
  { title: "Favorites", description: "Favorite Mode provides a separate shared bank of eight slots, independent of your four presets. You can switch to it here or with a hotkey.", target: ".sidebar-nav button:nth-child(2)", view: "slots" },
  { title: "Settings", description: "This is where you customize SPLIT. We'll walk through the main settings sections next.", target: ".sidebar-nav button:nth-child(3)", view: "settings", section: "general" },
  { title: "General settings", description: "Customize desktop behavior, appearance, startup sound, and other preferences here.", target: ".general-settings-section", view: "settings", section: "general" },
  ...(isBorderlessEdition ? [{ title: "Quick access", description: "In the Borderless edition, enable the Quick Access overlay and configure its position here. It lets you work with slots while in Deadlock.", target: ".quick-access-settings-group", view: "settings" as const, section: "general" as const }] : []),
  { title: "Discord rich presence", description: "Choose what Discord displays for your Hideout, Explore NYC, matchmaking, and matches. Customize hero images, text, and activity details.", target: ".drp-settings-shell", view: "settings", section: "discord" },
  { title: "Keyboard shortcuts", description: "Customize shortcuts for saving and loading slots, switching presets, favorites, and undo / redo.", tips: ["Some actions use mouse gestures instead of configurable hotkeys. Right-click a saved slot preview to open its screenshot."], target: ".settings-view", view: "settings", section: "hotkeys" },
  ...(isBorderlessEdition ? [{ title: "In-game notifications", description: "Choose where in-game notifications appear, how long they remain visible, and whether they use slot colors.", target: ".settings-view", view: "settings" as const, section: "notifications" as const }] : []),
  { title: "Diagnostics", description: "You don't need this section for everyday use. If something stops working, check these status indicators to identify the problem. Select the info icons on individual cards to learn what each one means.", target: ".settings-view", view: "settings", section: "diagnostics" },
  { title: "You're ready!", description: "You can reopen this guide at any time from Settings > About. Have fun practicing!", target: ".about-settings-section", view: "settings", section: "about" },
];

export function isOnboardingCompleted(): boolean {
  try { return window.localStorage.getItem(KEY) === "true"; } catch { return false; }
}
type Props = { onClose: () => void; navigate: (view: "slots" | "settings", section?: Section) => void };
type Rect = { top: number; left: number; right: number; bottom: number; width: number; height: number };
export default function Onboarding({ onClose, navigate }: Props) {
  const [index, setIndex] = useState(0);
  const [rect, setRect] = useState<Rect | null>(null);
  const popoverRef = useRef<HTMLElement>(null);
  const [popoverHeight, setPopoverHeight] = useState(300);
  useLayoutEffect(() => {
    const el = popoverRef.current;
    if (!el) return;
    const update = () => setPopoverHeight(el.getBoundingClientRect().height);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, [index]);
  const step = steps[index];
  useLayoutEffect(() => { navigate(step.view, step.section); }, [index, navigate, step.view, step.section]);
  useEffect(() => {
    let frame = 0;
    let attempts = 0;
    const update = () => {
      const el = step.target ? document.querySelector<HTMLElement>(step.target) : null;
      if (el) {
        if (attempts === 0) el.scrollIntoView({ block: "nearest", behavior: "instant" });
        let r = el.getBoundingClientRect();
        // DRP is a long settings page: spotlight its full content area,
        // not just the short header/preview panel.
        if (step.section === "discord") {
          const container = document.querySelector<HTMLElement>(".settings-view");
          if (container) r = container.getBoundingClientRect();
        }
        // Diagnostics starts with the separate Operational health summary,
        // above the diagnostic cards. Include both in the spotlight.
        if (step.section === "diagnostics") {
          const summary = document.querySelector<HTMLElement>(".health-summary");
          if (summary) {
            const top = summary.getBoundingClientRect();
            r = {
              ...r,
              top: Math.min(top.top, r.top),
              left: Math.min(top.left, r.left),
              right: Math.max(top.right, r.right),
              bottom: Math.max(top.bottom, r.bottom),
              width: Math.max(top.right, r.right) - Math.min(top.left, r.left),
              height: Math.max(top.bottom, r.bottom) - Math.min(top.top, r.top),
            } as DOMRect;
          }
        }
        if (r.width > 0 && r.height > 0) {
          const p = 7;
          setRect({ top: Math.max(0, r.top - p), left: Math.max(0, r.left - p), right: Math.min(innerWidth, r.right + p), bottom: Math.min(innerHeight, r.bottom + p), width: r.width + p * 2, height: r.height + p * 2 });
        } else setRect(null);
      } else setRect(null);
      if (++attempts < 45) frame = requestAnimationFrame(update);
    };
    update();
    const listener = () => { attempts = 44; update(); };
    window.addEventListener("resize", listener);
    window.addEventListener("scroll", listener, true);
    return () => { cancelAnimationFrame(frame); window.removeEventListener("resize", listener); window.removeEventListener("scroll", listener, true); };
  }, [index, step.target]);
  const finish = () => {
    try { window.localStorage.setItem(KEY, "true"); } catch { /* unavailable */ }
    onClose();
  };
  const w = window.innerWidth, h = window.innerHeight;
  const tipWidth = Math.min(370, w - 32);
  const usableHeight = Math.min(popoverHeight, h - 32);
  const gap = 18;
  const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
  const fits = (left: number, top: number) => !rect ||
    left + tipWidth + gap <= rect.left ||
    left >= rect.right + gap ||
    top + usableHeight + gap <= rect.top ||
    top >= rect.bottom + gap;
  const preferredLeft = rect ? clamp(rect.left, 16, w - tipWidth - 16) : (w - tipWidth) / 2;
  const preferredTop = rect ? clamp(rect.top, 16, h - usableHeight - 16) : (h - usableHeight) / 2;
  const candidates: Array<[number, number]> = rect ? [
    [rect.right + gap, preferredTop],
    [rect.left - tipWidth - gap, preferredTop],
    [preferredLeft, rect.bottom + gap],
    [preferredLeft, rect.top - usableHeight - gap],
    [16, 16],
    [w - tipWidth - 16, 16],
    [16, h - usableHeight - 16],
    [w - tipWidth - 16, h - usableHeight - 16],
  ] : [[preferredLeft, preferredTop]];
  const valid = candidates.filter(([left, top]) =>
    left >= 16 && top >= 16 && left + tipWidth <= w - 16 &&
    top + usableHeight <= h - 16 && fits(left, top)
  );
  // Large settings panels span almost the entire content area. Keep their
  // explanations anchored at the bottom, rather than covering the first controls.
  // Leave all other tour steps on the automatic positioning algorithm.
  const bottomAnchored = ["General settings", "Discord rich presence", "Keyboard shortcuts", "Diagnostics"].includes(step.title);
  const [tipLeft, tipTop] = bottomAnchored
    ? [16, Math.max(16, h - usableHeight - 16)]
    : (valid[0] ?? [
        clamp(preferredLeft, 16, w - tipWidth - 16),
        clamp(preferredTop, 16, h - usableHeight - 16),
      ]);
  return (
    <div className="tour-root" role="dialog" aria-modal="true" aria-labelledby="tour-title">
      {rect ? <>
        <div className="tour-shade" style={{ left: 0, top: 0, width: w, height: rect.top }} />
        <div className="tour-shade" style={{ left: 0, top: rect.bottom, width: w, height: h - rect.bottom }} />
        <div className="tour-shade" style={{ left: 0, top: rect.top, width: rect.left, height: rect.bottom - rect.top }} />
        <div className="tour-shade" style={{ left: rect.right, top: rect.top, width: w - rect.right, height: rect.bottom - rect.top }} />
        <div className="tour-highlight" style={{ top: rect.top, left: rect.left, width: rect.right - rect.left, height: rect.bottom - rect.top }} />
      </> : <div className="tour-shade" style={{ inset: 0 }} />}
      <section ref={popoverRef} className="tour-popover" style={{ left: tipLeft, top: tipTop, width: tipWidth }}>
        <div className="tour-heading"><strong>SPLIT <small>GUIDE</small></strong><button type="button" onClick={finish}>Skip guide</button></div>
        <div className="tour-progress">{steps.map((_, i) => <span key={i} className={i <= index ? "active" : ""} />)}</div>
        <p className="tour-counter">STEP {index + 1} OF {steps.length}</p>
        <h2 id="tour-title">{step.title}</h2>
        <p className="tour-description">{step.description}</p>
        {step.tips && <div className="tour-tips">{step.tips.map((tip, i) => <p key={i}><span className="tour-tip-label">TIP</span>{tip}</p>)}</div>}
        <div className="tour-actions"><span>{index + 1} / {steps.length}</span><div>{index > 0 && <button type="button" className="tour-back" onClick={() => setIndex(i => i - 1)}>Back</button>}<button type="button" className="tour-next" onClick={() => index === steps.length - 1 ? finish() : setIndex(i => i + 1)}>{index === steps.length - 1 ? "Finish" : "Next"}</button></div></div>
      </section>
    </div>
  );
}
