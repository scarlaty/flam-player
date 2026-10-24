/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import type { CompositionSummary, PackComposition } from "../../api/pack";
import { ToastProvider } from "../../components";
import { PACK_HASH } from "../../router";
import { expectNoAxeViolations } from "../../test/axe";
import { tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { CompositionsSection } from "./CompositionsSection";

tolerateLoadInAsyncUtils();

function summary(id: string, name: string, extra: Partial<CompositionSummary> = {}) {
  return {
    id,
    name,
    created_at: 1_790_000_000,
    last_used_at: 1_790_000_000,
    packs: 1,
    episodes: 3,
    feeds: 1,
    missing_feeds: 0,
    ...extra,
  };
}

const withEpisodes: PackComposition = {
  metadata: { title: "", subtitle: null, age: 0, description: null },
  cover: { kind: "default" },
  menu: [{ kind: "episode", source_id: "a", episode_id: "e1" }],
};

let list: CompositionSummary[];
let draft: PackComposition | null;
let notice: string | null;
let calls: { cmd: string; args: Record<string, any> }[];
let failures: Record<string, { kind: string; message: string }>;

beforeEach(() => {
  window.location.hash = "#/podcasts";
  list = [summary("c1", "Contes du soir"), summary("c2", "Comptines")];
  draft = null;
  notice = null;
  calls = [];
  failures = {};
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    const failure = failures[cmd];
    if (failure) throw failure;
    switch (cmd) {
      case "pack_compositions_list":
        return { compositions: list, notice, max: 50 };
      case "pack_draft_read":
        return { composition: draft, revision: 4, notice: null, report: null };
      case "pack_composition_open":
        return { saved: true, revision: 5 };
      case "pack_composition_rename":
        list = list.map((c) => (c.id === payload.id ? { ...c, name: payload.name } : c));
        return null;
      case "pack_composition_delete":
        list = list.filter((c) => c.id !== payload.id);
        return null;
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => clearMocks());

function renderSection() {
  return render(
    <ToastProvider>
      <CompositionsSection />
    </ToastProvider>,
  );
}

const commands = () => calls.map((c) => c.cmd);

it("liste les compositions avec leurs détails et les flux supprimés", async () => {
  list = [
    summary("c1", "Contes du soir", { missing_feeds: 1 }),
    summary("c2", "Comptines", { episodes: 1, last_used_at: 0 }),
  ];
  const { container } = renderSection();
  const items = await screen.findAllByRole("listitem");
  expect(items).toHaveLength(2);
  expect(within(items[0]!).getByText("Contes du soir")).toBeTruthy();
  expect(within(items[0]!).getByText(/3 épisodes · dernier pack le/)).toBeTruthy();
  expect(
    within(items[0]!).getByText("1 flux supprimé : ses épisodes sont indisponibles."),
  ).toBeTruthy();
  expect(within(items[1]!).getByText("1 épisode")).toBeTruthy();
  expect(screen.getByText("2 compositions sur 50 au plus.")).toBeTruthy();
  await expectNoAxeViolations(container);
});

it("liste vide : explique quand les compositions apparaissent", async () => {
  list = [];
  renderSection();
  expect(
    await screen.findByText(/elles apparaissent ici après la création d'un pack/),
  ).toBeTruthy();
});

it("affiche le message du backend (fichier mis de côté)", async () => {
  notice = "Le fichier des compositions enregistrées était illisible.";
  renderSection();
  expect(await screen.findByText(notice)).toBeTruthy();
});

it("échec de lecture : message et nouvelle tentative", async () => {
  failures.pack_compositions_list = { kind: "io", message: "disque absent" };
  renderSection();
  expect(await screen.findByText(/Impossible de lire les compositions/)).toBeTruthy();
  delete failures.pack_compositions_list;
  fireEvent.click(screen.getByRole("button", { name: "Réessayer" }));
  expect(await screen.findByText("Contes du soir")).toBeTruthy();
});

it("ouvrir sans brouillon en cours : copie et va à l'éditeur sans confirmation", async () => {
  renderSection();
  fireEvent.click(
    await screen.findByRole("button", {
      name: "Ouvrir « Contes du soir » dans l'éditeur",
    }),
  );
  await waitFor(() => expect(window.location.hash).toBe(PACK_HASH));
  expect(calls.find((c) => c.cmd === "pack_composition_open")?.args).toEqual({
    id: "c1",
  });
  expect(screen.queryByRole("alertdialog")).toBeNull();
});

it("ouvrir avec un brouillon non vide : confirmation, Annuler ne touche à rien", async () => {
  draft = withEpisodes;
  renderSection();
  fireEvent.click(
    await screen.findByRole("button", { name: "Ouvrir « Comptines » dans l'éditeur" }),
  );
  const dialog = await screen.findByRole("alertdialog");
  expect(within(dialog).getByText(/sera remplacé par « Comptines »/)).toBeTruthy();
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(commands()).not.toContain("pack_composition_open");
  expect(window.location.hash).toBe("#/podcasts");

  fireEvent.click(
    screen.getByRole("button", { name: "Ouvrir « Comptines » dans l'éditeur" }),
  );
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Remplacer",
    }),
  );
  await waitFor(() => expect(window.location.hash).toBe(PACK_HASH));
  expect(calls.find((c) => c.cmd === "pack_composition_open")?.args).toEqual({
    id: "c2",
  });
});

it("brouillon illisible : la confirmation est demandée par prudence", async () => {
  failures.pack_draft_read = { kind: "io", message: "illisible" };
  renderSection();
  fireEvent.click(
    await screen.findByRole("button", { name: "Ouvrir « Comptines » dans l'éditeur" }),
  );
  expect(await screen.findByRole("alertdialog")).toBeTruthy();
});

it("ouvrir une composition disparue : erreur affichée et liste relue", async () => {
  failures.pack_composition_open = {
    kind: "not_found",
    message: "composition introuvable",
  };
  renderSection();
  fireEvent.click(
    await screen.findByRole("button", {
      name: "Ouvrir « Contes du soir » dans l'éditeur",
    }),
  );
  expect(await screen.findByText("composition introuvable")).toBeTruthy();
  expect(window.location.hash).toBe("#/podcasts");
  await waitFor(() =>
    expect(commands().filter((c) => c === "pack_compositions_list")).toHaveLength(2),
  );
});

it("renommer : nom vérifié, puis enregistré et liste relue", async () => {
  renderSection();
  fireEvent.click(await screen.findByRole("button", { name: "Renommer « Comptines »" }));
  const dialog = await screen.findByRole("dialog");
  const input = within(dialog).getByRole("textbox", { name: /Nom/ });
  fireEvent.change(input, { target: { value: "   " } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Renommer" }));
  expect(await within(dialog).findByText("Le nom ne peut pas être vide.")).toBeTruthy();
  expect(commands()).not.toContain("pack_composition_rename");
  fireEvent.change(input, { target: { value: "  Chansons  " } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Renommer" }));
  expect(await screen.findByText("Composition renommée en « Chansons ».")).toBeTruthy();
  expect(calls.find((c) => c.cmd === "pack_composition_rename")?.args).toEqual({
    id: "c2",
    name: "Chansons",
  });
  expect(await screen.findByText("Chansons")).toBeTruthy();
});

it("renommer : refus du backend affiché dans le dialogue", async () => {
  failures.pack_composition_rename = {
    kind: "validation",
    message: "une autre composition porte déjà ce nom",
  };
  renderSection();
  fireEvent.click(await screen.findByRole("button", { name: "Renommer « Comptines »" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByRole("textbox", { name: /Nom/ }), {
    target: { value: "contes du soir" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Renommer" }));
  expect(
    await within(dialog).findByText("une autre composition porte déjà ce nom"),
  ).toBeTruthy();
});

it("supprimer : confirmation, puis liste relue", async () => {
  renderSection();
  fireEvent.click(
    await screen.findByRole("button", { name: "Supprimer « Contes du soir »" }),
  );
  const dialog = await screen.findByRole("alertdialog");
  expect(
    within(dialog).getByText(/histoires déjà créées ne sont pas touchées/),
  ).toBeTruthy();
  fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer" }));
  expect(
    await screen.findByText("Composition « Contes du soir » supprimée."),
  ).toBeTruthy();
  await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1));
  expect(calls.find((c) => c.cmd === "pack_composition_delete")?.args).toEqual({
    id: "c1",
  });
});

it("limite atteinte : le compteur invite à supprimer", async () => {
  list = Array.from({ length: 50 }, (_, i) => summary(`c${i}`, `P ${i}`));
  renderSection();
  expect(
    await screen.findByText(
      "50 compositions sur 50 au plus : supprimez-en une pour conserver les suivantes.",
    ),
  ).toBeTruthy();
});
