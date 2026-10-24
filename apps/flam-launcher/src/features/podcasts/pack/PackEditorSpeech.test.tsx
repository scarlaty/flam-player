/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DraftView, PackComposition, ValidationReport } from "../../../api/pack";
import { setPackRightsAck } from "../../settings/preferences";
import { PodcastSelectionProvider } from "../SelectionContext";
import { PackEditor } from "./PackEditor";

// Titres lus dans l'éditeur de pack (#462, PR 3) : case enregistrée dans le brouillon, aperçu du
// titre, erreurs `tts_*` de la création traduites.

vi.mock("../../../api/window", () => ({
  onWindowCloseRequested: () => Promise.resolve(() => {}),
}));

const composition: PackComposition = {
  metadata: { title: "", subtitle: null, age: 0, description: null },
  cover: { kind: "default" },
  menu: [{ kind: "episode", source_id: "a", episode_id: "1" }],
};

const report = (c: PackComposition): ValidationReport => ({
  valid: true,
  problems: [],
  unavailable: [],
  episodes: 1,
  feeds: 1,
  estimated_bytes: 1_000_000,
  unknown_size: 0,
  metadata: {
    title: c.metadata.title.trim() || "Contes du soir",
    subtitle: "",
    age: 0,
    description: null,
  },
  cover: null,
});

let draft: DraftView;
let calls: { cmd: string; args: Record<string, any> }[];
let createFailure: { kind: string; message: string } | null;

beforeEach(() => {
  draft = { composition, revision: 3, notice: null, report: report(composition) };
  calls = [];
  createFailure = null;
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    switch (cmd) {
      case "pack_draft_read":
        return draft;
      case "pack_draft_save":
        return { saved: true, revision: payload.revision };
      case "pack_validate":
        return report(payload.composition);
      case "podcast_feed":
        return {
          source_id: "a",
          feed: {
            title: "Contes du soir",
            description: null,
            copyright: null,
            has_image: false,
            episodes: [
              {
                id: "1",
                title: "Épisode 1",
                description: null,
                pub_date: null,
                duration: 600,
                author: null,
                enclosure_type: "audio/mpeg",
                enclosure_length: 1_000_000,
                has_image: false,
                explicit: false,
              },
            ],
            warnings: [],
          },
          status: { offline: false, updated_at: 1_790_000_000, error: null },
        };
      case "tts_voices":
        return {
          voices: [{ id: "v-h", name: "Microsoft Hortense", language: "fr-FR" }],
          default_id: "v-h",
        };
      case "settings_get":
        return { settings: { tts_voice: null } };
      case "tts_preview":
        return { played: true, duration_ms: 60_000 };
      case "tts_preview_stop":
        return null;
      case "pack_create":
        if (createFailure) throw createFailure;
        throw new Error("création non simulée");
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => {
  clearMocks();
  window.localStorage.clear();
});

function renderEditor() {
  return render(
    <PodcastSelectionProvider>
      <PackEditor saveDelayMs={20} validateDelayMs={20} />
    </PodcastSelectionProvider>,
  );
}

const lastSaved = () =>
  calls.filter((c) => c.cmd === "pack_draft_save").at(-1)?.args.composition as
    PackComposition | undefined;

it("la case des titres lus est enregistrée dans le brouillon", async () => {
  renderEditor();
  const box = await screen.findByRole("checkbox", {
    name: "Lire les titres à voix haute",
  });
  await screen.findByText(/Voix : Microsoft Hortense/);
  expect(box).not.toBeChecked();
  fireEvent.click(box);
  await waitFor(() => expect(lastSaved()?.titles_audio).toBe(true));
  fireEvent.click(box);
  await waitFor(() => expect(lastSaved()?.titles_audio).toBe(false));
});

it("l'aperçu lit le titre du pack (titre par défaut du flux, puis titre saisi)", async () => {
  renderEditor();
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  await waitFor(() =>
    expect(calls.find((c) => c.cmd === "tts_preview")?.args).toEqual({
      text: "Contes du soir",
      voice: "v-h",
    }),
  );
  // Le titre change : l'aperçu en cours est coupé.
  fireEvent.change(screen.getByRole("textbox", { name: /Titre/ }), {
    target: { value: "Mon pack" },
  });
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  await waitFor(() =>
    expect(calls.filter((c) => c.cmd === "tts_preview").at(-1)?.args.text).toBe(
      "Mon pack",
    ),
  );
  expect(calls.some((c) => c.cmd === "tts_preview_stop")).toBe(true);
});

it("création refusée faute de voix : message traduit et clair", async () => {
  setPackRightsAck();
  draft = {
    ...draft,
    composition: { ...composition, titles_audio: true },
  };
  createFailure = { kind: "tts_no_voice", message: "texte technique du backend" };
  renderEditor();
  const create = await screen.findByRole("button", { name: "Créer l'histoire" });
  await waitFor(() => expect(create).toBeEnabled());
  fireEvent.click(create);
  const alert = await screen.findByText(/Aucune voix française n'est installée/);
  expect(alert.textContent).toContain("décochez « Lire les titres à voix haute »");
  expect(screen.getByText("Aucune voix française")).toBeInTheDocument();
  expect(screen.queryByText("texte technique du backend")).toBeNull();
});
