import raw from "../src-tauri/tauri.conf.json?raw";
import { expect, it } from "vitest";

/*
 * Sous Windows, Tauri intercepte le glisser-déposer de la fenêtre pour le dépôt de fichiers
 * natif (`dragDropEnabled`, vrai par défaut) : les événements HTML5 (`draggable`, `ondrop`)
 * n'arrivent alors jamais à la page (#300). Le launcher n'utilise pas ce dépôt de fichiers
 * (les dossiers passent par le sélecteur natif) : il doit rester désactivé.
 */
it("désactive le dépôt de fichiers natif sur chaque fenêtre (glisser-déposer HTML5)", () => {
  const config = JSON.parse(raw) as { app: { windows: { dragDropEnabled?: boolean }[] } };
  expect(config.app.windows.length).toBeGreaterThan(0);
  for (const window of config.app.windows) {
    expect(window.dragDropEnabled).toBe(false);
  }
});
