import { expect, it } from "vitest";
import type { InstallTask } from "../../api/install";
import { makeEntry } from "../../test/entries";
import {
  describePhase,
  errorKindText,
  isActiveState,
  MAX_SHOWN_WARNINGS,
  taskErrorMessage,
  taskProgress,
  toPanelTask,
} from "./taskView";

function task(over: Partial<InstallTask>): InstallTask {
  return {
    id: "t",
    entry: makeEntry({ title: "Conte" }),
    state: "queued",
    percent: null,
    error: null,
    created_at: 0,
    pack: null,
    ...over,
  };
}

it("décrit l'étape en cours", () => {
  expect(describePhase("queued")).toBe("En attente");
  expect(describePhase("downloading")).toBe("Téléchargement");
  expect(describePhase("downloading", { received: 1024, total: 2048, phase: null })).toBe(
    "Téléchargement : 1 Ko sur 2 Ko",
  );
  expect(describePhase("converting", { received: null, total: null, phase: "zip" })).toBe(
    "Conversion : archive",
  );
  expect(describePhase("converting", { received: null, total: null, phase: "x" })).toBe(
    "Conversion : x",
  );
  expect(describePhase("installing")).toBe("Installation");
  expect(describePhase("done")).toBeUndefined();
});

it("distingue les tâches actives des tâches terminées", () => {
  expect(
    ["queued", "downloading", "converting", "installing"].every((s) =>
      isActiveState(s as never),
    ),
  ).toBe(true);
  expect(["done", "failed", "cancelled"].some((s) => isActiveState(s as never))).toBe(
    false,
  );
});

it("met une tâche au format du panneau", () => {
  expect(toPanelTask(task({ state: "downloading", percent: 12 }))).toMatchObject({
    title: "Conte",
    kind: "download",
    state: "running",
    progress: 12,
    phase: "Téléchargement",
  });
  expect(toPanelTask(task({ state: "converting" }))).toMatchObject({
    kind: "convert",
    state: "running",
    progress: undefined,
  });
  const failed = toPanelTask(
    task({
      state: "failed",
      error: { kind: "conversion", message: "Image manquante.", code: "missing_asset" },
    }),
  );
  expect(failed).toMatchObject({ state: "error", detail: "Image manquante." });
  expect(failed.phase).toBeUndefined();
  expect(toPanelTask(task({ state: "cancelled" })).state).toBe("cancelled");
});

it("donne l'avancement affiché sur la carte", () => {
  expect(taskProgress(task({ state: "converting", percent: 30 }))).toEqual({
    value: 30,
    text: "Conversion",
  });
  expect(taskProgress(task({ state: "queued" })).value).toBe("indeterminate");
});

it("décrit la préparation d'un pack et signale les épisodes retirés", () => {
  expect(describePhase("preparing")).toBe("Préparation du pack");
  expect(
    describePhase("preparing", { received: null, total: null, phase: "audio" }),
  ).toBe("Préparation du pack : conversion audio");
  expect(describePhase("prepared")).toBe("Pack préparé, en attente de conversion");
  const panel = toPanelTask(
    task({ state: "done", pack: { episodes: 3, dropped: ["Deux"], warnings: [] } }),
  );
  expect(panel.detail).toBe("1 épisode non inclus : Deux");
  expect(toPanelTask(task({ state: "preparing", percent: 40 })).state).toBe("running");
});

it("traduit unsupported_format en un message clair et non technique", () => {
  const error = {
    kind: "conversion",
    message: "conversion impossible (unsupported_format) : texte du backend",
    code: "unsupported_format",
  };
  const panel = toPanelTask(task({ state: "failed", error }));
  expect(panel.detail).toBe(
    "Cette histoire utilise un format que le launcher ne sait pas encore convertir (image non PNG ou archive compressée en bzip2/LZMA).",
  );
  expect(taskErrorMessage(error)).toBe(panel.detail);
  // Les autres codes (et les autres kinds) gardent le message du backend.
  expect(
    taskErrorMessage({ kind: "conversion", message: "m", code: "missing_asset" }),
  ).toBe("m");
  // Un code libre qui porte le nom d'une propriété héritée n'est jamais pris pour une clé.
  for (const code of [
    "constructor",
    "__proto__",
    "toString",
    "hasOwnProperty",
    "valueOf",
  ]) {
    expect(taskErrorMessage({ kind: "conversion", message: "m", code })).toBe("m");
  }
  expect(taskErrorMessage({ kind: "io", message: "disque", code: null })).toBe("disque");
  expect(taskErrorMessage({ kind: "io", message: "x", code: "unsupported_format" })).toBe(
    "x",
  );
});

it("traduit les échecs des titres lus (tts_*) par kind, sans clé héritée", () => {
  const failed = (kind: string, message = "texte du backend") =>
    toPanelTask(task({ state: "failed", error: { kind, message, code: null } })).detail;
  expect(failed("tts_no_voice")).toMatch(/^Aucune voix française n'est installée/);
  expect(failed("tts_no_voice")).toContain("Heure et langue > Voix");
  expect(failed("tts_unavailable")).toMatch(/^La synthèse vocale de Windows/);
  expect(failed("tts_failed")).toMatch(/^Les titres n'ont pas pu être lus à voix haute/);
  for (const kind of ["constructor", "__proto__", "toString", "task"]) {
    expect(failed(kind, "m")).toBe("m");
  }
  // Seul `tts_failed` ajoute le texte du backend sous le texte traduit.
  expect(failed("tts_no_voice")).not.toContain("texte du backend");
  expect(failed("tts_unavailable")).not.toContain("texte du backend");
  expect(failed("tts_failed")).toBe(
    `${errorKindText("tts_failed")}\nDétail : texte du backend`,
  );
  expect(errorKindText("hasOwnProperty")).toBeUndefined();
});

it("rétablit le détail du backend sous le texte traduit d'un échec tts_failed", () => {
  for (const message of [
    "La voix n'a pas répondu à temps.",
    "La synthèse vocale a échoué (E_FAIL).",
    "Plus de la moitié des titres n'ont pas pu être lus.",
  ]) {
    const error = { kind: "tts_failed", message, code: null };
    const [translated, detail, ...rest] = taskErrorMessage(error).split("\n");
    expect(translated).toBe(errorKindText("tts_failed"));
    expect(detail).toBe(`Détail : ${message}`);
    expect(rest).toEqual([]);
    // Même texte dans le panneau des tâches.
    expect(toPanelTask(task({ state: "failed", error })).detail).toBe(
      taskErrorMessage(error),
    );
  }
  // Message vide : pas de ligne de détail vide.
  expect(taskErrorMessage({ kind: "tts_failed", message: "  ", code: null })).toBe(
    errorKindText("tts_failed"),
  );
});

it("affiche les avertissements des titres lus d'un pack, les suivants comptés", () => {
  const panel = (dropped: string[], warnings: string[]) =>
    toPanelTask(task({ state: "done", pack: { episodes: 9, dropped, warnings } })).detail;
  expect(panel([], ["Voix remplacée."])).toBe("Voix remplacée.");
  expect(panel(["Deux"], ["« Un » : muet."])).toBe(
    "1 épisode non inclus : Deux « Un » : muet.",
  );
  const many = Array.from({ length: MAX_SHOWN_WARNINGS + 2 }, (_, i) => `W${i}.`);
  expect(panel([], many)).toBe("W0. W1. W2. Et 2 autres avertissements.");
  expect(panel([], many.slice(0, MAX_SHOWN_WARNINGS + 1))).toBe(
    "W0. W1. W2. Et 1 autre avertissement.",
  );
  expect(panel([], [])).toBeUndefined();
});
