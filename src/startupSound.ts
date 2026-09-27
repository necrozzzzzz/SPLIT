import { invoke } from "@tauri-apps/api/core";

import bootSoundUrl from "../assets/boot.wav?url";

export type StartupSoundSettings = {
  enabled: boolean;
  volume: number;
};

export type StartupSoundLaunch = {
  settings: StartupSoundSettings;
  shouldPlay: boolean;
};

let audio: HTMLAudioElement | null = null;
let launchClaim: Promise<StartupSoundLaunch> | null = null;
let startupPlaybackHandled = false;

function player(): HTMLAudioElement {
  if (!audio) {
    audio = new Audio(bootSoundUrl);
    audio.preload = "auto";
  }

  return audio;
}

export function claimStartupSound(): Promise<StartupSoundLaunch> {
  launchClaim ??= invoke<StartupSoundLaunch>("claim_startup_sound");
  return launchClaim;
}

export async function playStartupSound(volume: number): Promise<void> {
  const current = player();
  current.pause();
  current.currentTime = 0;
  current.volume = Math.min(100, Math.max(0, volume)) / 100;
  await current.play();
}

export async function playClaimedStartupSound(
  launch: StartupSoundLaunch,
): Promise<void> {
  if (startupPlaybackHandled) return;
  startupPlaybackHandled = true;

  if (!launch.shouldPlay || !launch.settings.enabled) return;

  try {
    await playStartupSound(launch.settings.volume);
  } catch {
    // Startup audio is cosmetic and must never interfere with app startup.
  }
}
