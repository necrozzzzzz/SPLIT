export type SplitEdition = "borderless" | "panorama";

export const splitEdition: SplitEdition = __SPLIT_EDITION__;
export const isBorderlessEdition = splitEdition === "borderless";
export const isPanoramaEdition = splitEdition === "panorama";
export const splitEditionLabel = isBorderlessEdition
  ? "Borderless"
  : "Panorama";
