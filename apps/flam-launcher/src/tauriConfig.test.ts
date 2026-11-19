import raw from "../src-tauri/tauri.conf.json?raw";
import { expect, it } from "vitest";

interface WindowConfig {
  label?: string;
  create?: boolean;
  dragDropEnabled?: boolean;
}

/** Modèle de la fenêtre de l'éditeur d'histoires (`ops_commands::EDITOR_TEMPLATE`). */
const EDITOR = "editor";

const windows = (JSON.parse(raw) as { app: { windows: WindowConfig[] } }).app.windows;

/*
 * Sous Windows, Tauri intercepte le glisser-déposer de la fenêtre pour le dépôt de fichiers
 * natif (`dragDropEnabled`, vrai par défaut) : les événements HTML5 (`draggable`, `ondrop`)
 * n'arrivent alors jamais à la page (#300). Le launcher n'utilise pas ce dépôt de fichiers
 * (les dossiers passent par le sélecteur natif) : il doit rester désactivé, sauf dans la
 * fenêtre de l'éditeur d'histoires (docs/creation-dossier.md § 4.3, Q6), qui reçoit les fichiers
 * déposés depuis l'Explorateur et n'utilise pas le glisser-déposer HTML5.
 */
it("désactive le dépôt de fichiers natif sur chaque fenêtre sauf l'éditeur (#300)", () => {
  const others = windows.filter((window) => window.label !== EDITOR);
  expect(others.length).toBeGreaterThan(0);
  for (const window of others) {
    expect(window.dragDropEnabled).toBe(false);
  }
});

it("active le dépôt de fichiers sur le modèle de l'éditeur, jamais créé au démarrage", () => {
  const editor = windows.filter((window) => window.label === EDITOR);
  expect(editor).toHaveLength(1);
  expect(editor[0]?.dragDropEnabled).toBe(true);
  expect(editor[0]?.create).toBe(false);
});
