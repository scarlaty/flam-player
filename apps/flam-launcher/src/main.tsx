import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { isEditorWindow } from "./api/editor";
import { EditorApp } from "./features/authoring/editor/EditorApp";
import { applyRememberedTheme } from "./theme/theme";
import "./styles.css";

// Avant le premier rendu : évite un bref affichage au thème système au démarrage.
applyRememberedTheme();

const root = document.getElementById("root");
if (!root) {
  throw new Error("Élément #root introuvable");
}

// Fenêtre de l'éditeur d'histoires (`editor-<n>`) : sa propre racine, sans les fournisseurs
// de la fenêtre principale.
createRoot(root).render(
  <StrictMode>{isEditorWindow() ? <EditorApp /> : <App />}</StrictMode>,
);
