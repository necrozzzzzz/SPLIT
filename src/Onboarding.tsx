import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { isBorderlessEdition } from "./edition";
import { getLanguage, tourTitle, tourControl, tourDescription, tourDemoText, tourTips, runtimeUiText } from "./i18n";
import "./onboarding.css";

const KEY = "split.onboarding.v2.completed";
type Section = "general" | "discord" | "hotkeys" | "notifications" | "diagnostics" | "about";
type Step = { title: string; description: string; target: string; view: "slots" | "settings"; section?: Section; tips?: string[] };
const steps: Step[] = [
  { title: "Welcome to SPLIT", description: "Your Deadlock practice companion. This short interactive tour will show you where to find the tools you need. Nothing will be saved or loaded during the tour.", target: "", view: "slots" },
  { title: "Your presets", description: "Start here to organize your practice positions. You have four presets, each with eight independent slots. You can rename and switch between presets.", target: ".sidebar-nav button:first-child", view: "slots" },
  { title: "Save State slots", description: "Each card represents a saved position. Save your current location, then load it to return to that spot in Deadlock. Slots can have names, colors and screenshots.", target: ".slots-grid .slot-card:first-child", view: "slots", tips: ["Right-click Mina’s preview to try the screenshot viewer.", "Click the example slot name below to try renaming it."] },
  { title: "Slot actions", description: "Open the three-dot menu on a slot to find its actions, including renaming, clearing, copying a saved position, and assigning a color. Click the highlighted three-dot button on Mina’s example slot, then choose a color to try it in Tint mode.", target: ".slots-grid .slot-card:first-child .slot-card-menu", view: "slots" },
  { title: "Undo and redo", description: "Use these buttons to undo or redo supported changes to your saved positions. Their keyboard shortcuts are shown beside them.", target: ".header-history-actions", view: "slots" },
  { title: "Favorites", description: "Favorite Mode provides a separate shared bank of eight slots, independent of your four presets. You can switch to it here or with a hotkey.", target: ".sidebar-nav button:nth-child(2)", view: "slots" },
  { title: "Settings", description: "This is where you customize SPLIT. We'll walk through the main settings sections next.", target: ".sidebar-nav button:nth-child(3)", view: "settings", section: "general" },
  { title: "General settings", description: "Customize desktop behavior, appearance, startup sound, and other preferences here.", target: ".general-settings-section", view: "settings", section: "general" },
  ...(isBorderlessEdition ? [{ title: "Quick access", description: "In the Borderless edition, enable the Quick Access overlay and configure its position here. It lets you work with slots while in Deadlock.", target: ".quick-access-settings-group", view: "settings" as const, section: "general" as const }] : []),
  { title: "Discord rich presence", description: "Choose what Discord displays for your Hideout, Explore NYC, matchmaking, and matches. Customize hero images, text, and activity details.", target: ".drp-settings-shell", view: "settings", section: "discord" },
  { title: "Keyboard shortcuts", description: "Customize shortcuts for saving and loading slots, switching presets, favorites, and undo / redo.", target: ".settings-view", view: "settings", section: "hotkeys" },
  ...(isBorderlessEdition ? [{ title: "In-game notifications", description: "Choose where in-game notifications appear, how long they remain visible, and whether they use slot colors.", target: ".settings-view", view: "settings" as const, section: "notifications" as const }] : []),
  { title: "Diagnostics", description: "You don't need this section for everyday use. If something stops working, check these status indicators to identify the problem. Select the info icons on individual cards to learn what each one means.", target: ".settings-view", view: "settings", section: "diagnostics" },
  { title: "You're ready!", description: "You can reopen this guide at any time from Settings > About. Have fun practicing!", target: ".about-settings-section", view: "settings", section: "about" },
];


// Tour text is keyed by its original title so targeting and demo behavior remain unchanged.
const frenchTour: Record<string, { title: string; description: string; tips?: string[] }> = {
  "Welcome to SPLIT": { title: "Bienvenue dans SPLIT", description: "Votre compagnon d’entraînement sur Deadlock. Cette visite interactive vous présente les outils essentiels. Aucune position ne sera enregistrée ou chargée pendant le tutoriel." },
  "Your presets": { title: "Vos presets", description: "Organisez vos slots ici. Vous avez quatre presets de huit slots chacun. Renommez-les et passez de l’un à l’autre quand vous voulez." },
  "Save State slots": { title: "Emplacements de sauvegarde", description: "Chaque slot garde un endroit de la map en mémoire. Sauvegardez votre position, puis rechargez le slot pour y retourner dans Deadlock. Vous pouvez aussi lui donner un nom, une couleur et une capture d’écran.", tips: ["Faites un clic droit sur l’image de Mina pour voir la capture en grand.", "Cliquez sur le nom du slot pour le renommer."] },
  "Slot actions": { title: "Actions des emplacements", description: "Le menu à trois points d’un slot permet de le renommer, de le vider, de copier sa position ou de changer sa couleur. Essayez sur le slot de Mina : cliquez sur les trois points, puis choisissez une couleur." },
  "Undo and redo": { title: "Annuler et rétablir", description: "Une mauvaise manip ? Utilisez ces boutons pour annuler ou rétablir vos changements. Les raccourcis sont juste à côté." },
  "Favorites": { title: "Favoris", description: "Les favoris, c’est huit slots en plus, séparés de vos quatre presets. Ouvrez-les ici ou avec un raccourci clavier." },
  "Settings": { title: "Paramètres", description: "Réglez SPLIT à votre façon. On fait le tour des options principales." },
  "General settings": { title: "Paramètres généraux", description: "Choisissez comment SPLIT se comporte sur votre PC : apparence, son de démarrage et autres options." },
  "Quick access": { title: "Accès rapide", description: "En mode Borderless, activez l’accès rapide pour gérer vos slots sans quitter Deadlock. Vous pouvez aussi choisir où il s’affiche." },
  "Discord rich presence": { title: "Présence Discord", description: "Choisissez les informations affichées sur Discord dans le Hideout, Explore NYC, pendant la recherche de partie et en match. Personnalisez les portraits, les textes et les détails d’activité." },
  "Keyboard shortcuts": { title: "Raccourcis clavier", description: "Choisissez vos touches pour sauvegarder ou charger un slot, changer de preset, ouvrir les favoris ou annuler une action." },
  "In-game notifications": { title: "Notifications en jeu", description: "Choisissez où les notifications apparaissent en jeu, leur durée d’affichage et si elles utilisent les couleurs des emplacements." },
  "Diagnostics": { title: "Diagnostics", description: "Un truc ne marche pas ? Regardez les indicateurs ici pour voir ce qui coince. Cliquez sur les icônes d’info pour en savoir plus." },
  "You're ready!": { title: "Vous êtes prêt !", description: "Vous pourrez rouvrir ce tutoriel à tout moment depuis Paramètres > À propos. Bon entraînement !" },
};

export function isOnboardingCompleted(): boolean {
  try { return window.localStorage.getItem(KEY) === "true"; } catch { return false; }
}
type Props = { onClose: () => void; navigate: (view: "slots" | "settings", section?: Section) => void };
type Rect = { top: number; left: number; right: number; bottom: number; width: number; height: number };
export default function Onboarding({ onClose, navigate }: Props) {
  const [index, setIndex] = useState(0);
  const [demoOpen, setDemoOpen] = useState(false);
  const [demoSlotRect, setDemoSlotRect] = useState<Rect | null>(null);
  const [demoCardRect, setDemoCardRect] = useState<Rect | null>(null);
  const [demoMenuOpen, setDemoMenuOpen] = useState(false);
  const [demoColor, setDemoColor] = useState<string | null>(null);
  const [demoName, setDemoName] = useState(() => `${runtimeUiText("Slot", getLanguage())} 1`);
  const [demoRenaming, setDemoRenaming] = useState(false);
  const [demoDraft, setDemoDraft] = useState(() => `${runtimeUiText("Slot", getLanguage())} 1`);
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
  const [language, setTourLanguage] = useState(getLanguage);
  useEffect(() => { const update = () => setTourLanguage(getLanguage()); window.addEventListener("split-language-change", update); return () => window.removeEventListener("split-language-change", update); }, []);
  const translated = language === "fr" ? frenchTour[step.title] : undefined;
  const label = (en: string, fr: string) => {
    const demo = tourDemoText(en, language);
    if (demo !== en) return demo;
    const control = tourControl(en, language);
    return language === "fr" && control === en ? fr : control;
  };
  useEffect(() => { setDemoOpen(false); setDemoMenuOpen(false); setDemoColor(null); setDemoName(`${runtimeUiText("Slot", language)} 1`); setDemoDraft(`${runtimeUiText("Slot", language)} 1`); setDemoRenaming(false); }, [index, language]);
  useEffect(() => {
    if (!demoOpen) return;
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") { event.stopImmediatePropagation(); setDemoOpen(false); } };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [demoOpen]);
  useLayoutEffect(() => { navigate(step.view, step.section); }, [index, navigate, step.view, step.section]);
  useEffect(() => {
    let frame = 0;
    let attempts = 0;
    const update = () => {
      const el = step.target ? document.querySelector<HTMLElement>(step.target) : null;
      if (step.title === "Save State slots" || step.title === "Slot actions") {
        const card = document.querySelector<HTMLElement>(".slots-grid .slot-card:first-child");
        if (card) { const cr = card.getBoundingClientRect(); setDemoCardRect({ top: cr.top, left: cr.left, right: cr.right, bottom: cr.bottom, width: cr.width, height: cr.height }); } else setDemoCardRect(null);
        const preview = document.querySelector<HTMLElement>(".slots-grid .slot-card:first-child .slot-preview");
        if (preview) {
          const pr = preview.getBoundingClientRect();
          setDemoSlotRect({ top: pr.top, left: pr.left, right: pr.right, bottom: pr.bottom, width: pr.width, height: pr.height });
        } else { setDemoSlotRect(null); setDemoCardRect(null); }
      } else setDemoSlotRect(null);
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
    // Discord settings load asynchronously. Their height changes after the
    // initial tour measurement; observe layout changes instead of waiting for
    // the user to scroll before refreshing the spotlight.
    const resizeObserver = new ResizeObserver(() => update());
    const observed = [
      document.querySelector<HTMLElement>(".settings-view"),
      document.querySelector<HTMLElement>(".drp-settings-shell"),
    ].filter((el): el is HTMLElement => el !== null);
    observed.forEach((el) => resizeObserver.observe(el));
    const listener = () => { attempts = 44; update(); };
    window.addEventListener("resize", listener);
    window.addEventListener("scroll", listener, true);
    return () => { resizeObserver.disconnect(); cancelAnimationFrame(frame); window.removeEventListener("resize", listener); window.removeEventListener("scroll", listener, true); };
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
      {step.title === "Save State slots" && demoSlotRect && <div className="tour-live-preview" style={{ position: "fixed", left: demoSlotRect.left, top: demoSlotRect.top, width: demoSlotRect.width, height: demoSlotRect.height }} onContextMenu={(event) => { event.preventDefault(); setDemoOpen(true); }} title={tourDemoText("Right-click to view the example screenshot", language)}>
        <img src="/tutorial-mina.jpg" alt={tourDemoText("Example Deadlock screenshot featuring Mina", language)} />
        <span className="tour-live-preview-number">01</span>
        <span className="tour-live-preview-hint">{label("RIGHT-CLICK TO PREVIEW", "CLIC DROIT POUR AFFICHER")}</span>
      </div>}
      {step.title === "Save State slots" && demoCardRect && <div className="tour-live-rename" style={{ position: "fixed", left: demoCardRect.left, top: demoSlotRect ? demoSlotRect.bottom : demoCardRect.bottom - 56, width: demoCardRect.width, height: demoSlotRect ? Math.max(42, demoCardRect.bottom - demoSlotRect.bottom) : 56 }}>
        {demoRenaming ? <input autoFocus aria-label={tourDemoText("Rename example slot", language)} maxLength={40} value={demoDraft} onChange={event => setDemoDraft(event.target.value)} onKeyDown={event => { if (event.key === "Enter") { setDemoName(demoDraft.trim() || `${runtimeUiText("Slot", language)} 1`); setDemoRenaming(false); } if (event.key === "Escape") { event.stopPropagation(); setDemoRenaming(false); setDemoDraft(demoName); } }} onBlur={() => { setDemoName(demoDraft.trim() || `${runtimeUiText("Slot", language)} 1`); setDemoRenaming(false); }} /> : <button type="button" className="tour-live-rename-button" onClick={() => { setDemoDraft(demoName); setDemoRenaming(true); }} title={tourDemoText("Click to rename this example slot", language)}>{demoName}</button>}
        <small>{demoRenaming ? tourDemoText("Press Enter to confirm · Esc to cancel", language) : tourDemoText("CLICK THE NAME TO TRY RENAMING", language)}</small>
      </div>}
      {step.title === "Slot actions" && demoCardRect && <div className="tour-demo-slot-card" style={{ left: demoCardRect.left, top: demoCardRect.top, width: demoCardRect.width, height: demoCardRect.height }}>
        <div className="tour-demo-slot-image">
          <img src="/tutorial-mina.jpg" alt={tourDemoText("Example Deadlock save state featuring Mina", language)} />
          {demoColor && <div className="tour-demo-slot-tint" style={{ backgroundColor: demoColor }} />}
          <span>01</span>
        </div>
        <div className="tour-demo-slot-footer"><strong>{tourDemoText("Example · Mina", language)}</strong><small>{tourDemoText(demoColor ? "Tint applied! Try another color." : "Try the slot actions above", language)}</small></div>
        {!demoMenuOpen && <span className="tour-demo-slot-hint">{label("CLICK ··· TO TRY COLORS", "CLIQUEZ SUR ··· POUR TESTER LES COULEURS")}</span>}
        <button className="tour-demo-slot-menu-trigger" type="button" aria-label={tourDemoText("Open example slot actions", language)} aria-expanded={demoMenuOpen} onClick={() => setDemoMenuOpen(v => !v)}>···</button>
        {demoMenuOpen && <div className="slot-card-menu-panel tour-demo-slot-menu">
          <button type="button" className="slot-menu-item" onClick={() => setDemoMenuOpen(false)}>{tourDemoText("Rename", language)} <span>{tourDemoText("Example", language)}</span></button>
          <button type="button" className="slot-menu-item" onClick={() => setDemoMenuOpen(false)}>{tourDemoText("View screenshot", language)} <span>{tourDemoText("Right click", language)}</span></button>
          <div className="slot-menu-divider" />
          <span className="slot-menu-label">{tourDemoText("Color", language)}</span>
          <div className="slot-menu-colors">
            {([["None", null], ["Cyan", "#4fd1c5"], ["Yellow", "#ffd166"], ["Red", "#d98c8c"], ["Purple", "#9b8cff"], ["Green", "#62ff8f"]] as const).map(([label, color]) => <button key={label} type="button" className={`slot-menu-color ${demoColor === color ? "active" : ""}`} title={tourDemoText(label, language)} aria-label={`${tourDemoText(label, language)} · ${runtimeUiText("Slot color", language)}`} onClick={() => { setDemoColor(color); setDemoMenuOpen(false); }}><span style={{ width: 11, height: 11, borderRadius: 3, background: color ?? "#344b4b", display: "block" }} /></button>)}
          </div>
          <small className="tour-demo-disclaimer">{label("Demo only — no saved data changes", "Démonstration : aucune sauvegarde modifiée")}</small>
        </div>}
      </div>}
      {step.title === "Save State slots" && demoOpen && <div className="screenshot-viewer-backdrop tour-example-viewer" onClick={() => setDemoOpen(false)}>
        <section className="screenshot-viewer" role="dialog" aria-modal="true" aria-label={tourDemoText("Example screenshot for Mina", language)} onClick={(event) => event.stopPropagation()}>
          <header className="screenshot-viewer-header">
            <div><strong>{tourDemoText("Example slot · Mina", language)}</strong><span>{label("Example screenshot", "Capture d’écran d’exemple")}</span></div>
            <button type="button" aria-label={tourDemoText("Close screenshot", language)} title={tourDemoText("Close", language)} onClick={() => setDemoOpen(false)}>×</button>
          </header>
          <div className="screenshot-viewer-image"><img src="/tutorial-mina.jpg" alt={tourDemoText("Deadlock gameplay screenshot featuring Mina", language)} draggable={false} /></div>
          <footer className="screenshot-viewer-footer">{label("Example only — your saved positions are unchanged · Esc to close", "Exemple uniquement : vos sauvegardes restent intactes · Échap pour fermer")}</footer>
        </section>
      </div>}
      <section ref={popoverRef} className="tour-popover" style={{ left: tipLeft, top: tipTop, width: tipWidth }}>
        <div className="tour-heading"><strong>SPLIT <small>{tourControl("GUIDE", language)}</small></strong><button type="button" onClick={finish}>{label("Skip guide", "Passer le tutoriel")}</button></div>
        <div className="tour-progress">{steps.map((_, i) => <span key={i} className={i <= index ? "active" : ""} />)}</div>
        <p className="tour-counter">{label("STEP", "ÉTAPE")} {index + 1} {label("OF", "SUR")} {steps.length}</p>
        <h2 id="tour-title">{translated?.title ?? tourTitle(step.title, language)}</h2>
        <p className="tour-description">{translated?.description ?? tourDescription(step.title, step.description, language)}</p>
        {step.tips && <div className="tour-tips">{(translated?.tips ?? tourTips[language] ?? step.tips).map((tip, i) => <p key={i}><span className="tour-tip-label">{label("TIP", "ASTUCE")}</span>{tip}</p>)}</div>}
        <div className="tour-actions"><span>{index + 1} / {steps.length}</span><div>{index > 0 && <button type="button" className="tour-back" onClick={() => setIndex(i => i - 1)}>{label("Back", "Retour")}</button>}<button type="button" className="tour-next" onClick={() => index === steps.length - 1 ? finish() : setIndex(i => i + 1)}>{index === steps.length - 1 ? label("Finish", "Terminer") : label("Next", "Suivant")}</button></div></div>
      </section>
    </div>
  );
}
