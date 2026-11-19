import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import type { FolderProblem, FolderReport } from "../../api/authoring";
import { CreateStoryFromFolder } from "./CreateStoryFromFolder";
import { errorText, problemText, shownName } from "./problemText";

afterEach(() => clearMocks());

function report(patch: Partial<FolderReport> = {}): FolderReport {
  return {
    folder: "C:\\Histoires\\Loup",
    title: "Le loup",
    root: {
      kind: "menu",
      id: "",
      dir: "Loup",
      title: "Le loup",
      mode: "recueil",
      children: [],
    },
    problems: [],
    blocked: false,
    story_count: 3,
    audio_bytes: 100,
    mode: "recueil",
    suggested_mode: "recueil",
    uuid: null,
    installed_version: null,
    remembered: false,
    conflict: null,
    ...patch,
  };
}

function problem(
  code: string,
  path: string,
  severity: FolderProblem["severity"],
): FolderProblem {
  return { code, path, severity, message: "détail technique" };
}

it("dit les problèmes par leur code, sans le message technique", () => {
  expect(shownName("Animaux/01 Le loup.png")).toBe("01 Le loup");
  expect(problemText(problem("orphan_image", "Animaux/Le loup.png", "warning"))).toBe(
    "Image « Le loup.png » sans fichier audio du même nom : ignorée.",
  );
  expect(problemText(problem("no_story", "", "blocking"))).toBe(
    "Aucun fichier audio dans ce dossier.",
  );
  expect(problemText(problem("code_inconnu", "x", "warning"))).not.toContain("technique");
  expect(errorText({ kind: "io", message: "os error 5" })).toBe(
    "La création n'a pas pu être lancée. Vérifiez le dossier, puis réessayez.",
  );
});

it("choisir un dossier, voir ce qu'il contient, puis créer l'histoire", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "plugin:dialog|open") return "C:\\Histoires\\Loup";
    if (cmd === "authoring_read")
      return report({ problems: [problem("orphan_image", "Le chat.png", "warning")] });
    if (cmd === "authoring_create")
      return {
        task: {
          id: "t",
          entry: {},
          state: "queued",
          percent: null,
          error: null,
          created_at: 0,
          pack: null,
        },
        uuid: "u",
        version: 1,
        created: true,
        saved_in: "folder",
      };
    return null;
  });
  render(<CreateStoryFromFolder />);
  fireEvent.click(
    screen.getByRole("button", { name: "Créer une histoire depuis un dossier" }),
  );
  expect(await screen.findByText("3 histoires")).toBeInTheDocument();
  expect(
    screen.getByText("Image « Le chat.png » sans fichier audio du même nom : ignorée."),
  ).toBeInTheDocument();
  expect(screen.queryByText("détail technique")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Créer l'histoire" }));
  await waitFor(() => expect(calls.some((c) => c.cmd === "authoring_create")).toBe(true));
  expect(calls.find((c) => c.cmd === "authoring_create")?.args).toMatchObject({
    path: "C:\\Histoires\\Loup",
    choice: null,
  });
  await waitFor(() => expect(screen.queryByText("3 histoires")).toBeNull());
});

it("un dossier à arranger n'a pas de bouton de création", async () => {
  mockIPC((cmd) =>
    cmd === "authoring_read"
      ? report({ blocked: true, problems: [problem("no_story", "", "blocking")] })
      : null,
  );
  render(<CreateStoryFromFolder folder="C:\\Histoires\\Vide" />);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser" }));
  expect(
    await screen.findByText("Aucun fichier audio dans ce dossier."),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Créer l'histoire" })).toBeNull();
});

it("dossier copié : nouvelle histoire par défaut, ou remplacer l'autre", async () => {
  let created: unknown = null;
  mockIPC((cmd, args) => {
    if (cmd === "authoring_read")
      return report({
        uuid: "u",
        installed_version: 2,
        conflict: { kind: "copied", other_folder: "C:\\Autre", title: "Le loup" },
      });
    if (cmd === "authoring_create") {
      created = args;
      throw {
        kind: "authoring",
        code: "busy",
        message: "détail technique",
      };
    }
    return null;
  });
  render(<CreateStoryFromFolder folder="C:\\Histoires\\Loup" />);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser" }));
  fireEvent.click(await screen.findByLabelText("Remplacer « Le loup »"));
  fireEvent.click(screen.getByRole("button", { name: "Actualiser l'histoire" }));
  expect(
    await screen.findByText(
      "Cette histoire est déjà en cours de création : attendez la fin ou annulez-la.",
    ),
  ).toBeInTheDocument();
  expect(created).toMatchObject({ choice: "replace_other" });
});

it("erreurs reconnues par leur code, jamais par leur message", () => {
  const authoring = (code: string, message = "ne peut pas être le dossier, lien") => ({
    kind: "authoring",
    code,
    message,
  });
  expect(errorText(authoring("folder_missing"))).toBe(
    "Dossier introuvable : il a peut-être été déplacé ou renommé.",
  );
  expect(errorText(authoring("busy"))).toContain("déjà en cours de création");
  expect(errorText(authoring("conflict_copied"))).toContain("copié d'un autre dossier");
  expect(errorText(authoring("uuid_mismatch"))).toContain(
    "n'est pas celui de cette histoire",
  );
  expect(errorText(authoring("folder_changed"))).toContain("Actualiser");
  // Un message qui ressemble à un refus, sans code : texte générique.
  expect(errorText({ kind: "validation", message: "en cours de création, lien" })).toBe(
    "La création n'a pas pu être lancée. Vérifiez le dossier, puis réessayez.",
  );
});

it("textes dédiés : même nom et réglage « fin » sans effet", () => {
  expect(
    problemText(problem("duplicate_story_name", "Animaux/Le loup.mp3", "warning")),
  ).toBe(
    "« Le loup.mp3 » : un autre fichier audio porte le même nom, image et titre lu partagés.",
  );
  expect(problemText(problem("fin_ignored", "_histoire.json", "warning"))).toBe(
    "Réglage « fin » sans effet dans ce mode : retour au menu.",
  );
});

it("dossier introuvable : Choisir le dossier, puis le relire", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  let moved = false;
  let relocateFailure: unknown = {
    kind: "authoring",
    code: "uuid_mismatch",
    message: "détail technique",
  };
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "plugin:dialog|open") return "C:\\Rangé\\Loup";
    if (cmd === "authoring_read") {
      if (!moved)
        throw { kind: "authoring", code: "folder_missing", message: "détail technique" };
      return report({ folder: "C:\\Rangé\\Loup", uuid: "u", installed_version: 1 });
    }
    if (cmd === "authoring_relocate") {
      if (relocateFailure) throw relocateFailure;
      moved = true;
      return "C:\\Rangé\\Loup";
    }
    return null;
  });
  render(<CreateStoryFromFolder folder="C:\\Histoires\\Loup" uuid="u" />);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser" }));
  const dialog = await screen.findByRole("dialog", { name: "Dossier introuvable" });
  // Mauvais dossier : message clair, le dialogue reste ouvert.
  fireEvent.click(screen.getByRole("button", { name: "Choisir le dossier" }));
  expect(
    await screen.findByText(/n'est pas celui de cette histoire/),
  ).toBeInTheDocument();
  expect(dialog).toBeInTheDocument();
  relocateFailure = null;
  fireEvent.click(screen.getByRole("button", { name: "Choisir le dossier" }));
  expect(await screen.findByText("3 histoires")).toBeInTheDocument();
  expect(calls.filter((c) => c.cmd === "authoring_relocate").at(-1)?.args).toMatchObject({
    uuid: "u",
    path: "C:\\Rangé\\Loup",
  });
  expect(calls.filter((c) => c.cmd === "authoring_read").at(-1)?.args).toMatchObject({
    path: "C:\\Rangé\\Loup",
  });
  expect(screen.queryByRole("dialog", { name: "Dossier introuvable" })).toBeNull();
});

it("sans identifiant, un dossier introuvable est seulement signalé", async () => {
  mockIPC((cmd) => {
    if (cmd === "authoring_read")
      throw { kind: "authoring", code: "folder_missing", message: "détail technique" };
    return null;
  });
  render(<CreateStoryFromFolder folder="C:\\Histoires\\Loup" />);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser" }));
  expect(
    await screen.findByText(
      "Dossier introuvable : il a peut-être été déplacé ou renommé.",
    ),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Choisir le dossier" })).toBeNull();
});
