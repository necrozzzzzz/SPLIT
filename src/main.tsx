import {
  StrictMode,
} from "react";

import {
  createRoot,
} from "react-dom/client";

import App from "./App";
import QuickAccess from "./QuickAccess";

import "./styles.css";


const quickAccess =
  new URLSearchParams(
    window.location.search,
  ).get("quick-access") ===
  "1";

document.documentElement.classList.toggle(
  "quick-access-page",
  quickAccess,
);

document.body.classList.toggle(
  "quick-access-page",
  quickAccess,
);

document
  .getElementById("root")
  ?.classList.toggle(
    "quick-access-root",
    quickAccess,
  );


createRoot(
  document.getElementById(
    "root",
  )!,
).render(
  <StrictMode>
    {quickAccess
      ? <QuickAccess />
      : <App />}
  </StrictMode>,
);
