import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyRememberedTheme } from "./theme/theme";
import "./styles.css";

// Avant le premier rendu : évite un bref affichage au thème système au démarrage.
applyRememberedTheme();

const root = document.getElementById("root");
if (!root) {
  throw new Error("Élément #root introuvable");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
