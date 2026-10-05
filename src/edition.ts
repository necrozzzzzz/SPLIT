export type SplitEdition = "borderless" | "fullscreen";

export const splitEdition: SplitEdition = __SPLIT_EDITION__;
export const isBorderlessEdition = splitEdition === "borderless";
export const isFullscreenEdition = splitEdition === "fullscreen";
export const splitEditionLabel = isBorderlessEdition
  ? "Borderless"
  : "Fullscreen";
