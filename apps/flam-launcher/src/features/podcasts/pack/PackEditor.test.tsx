/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect, useState, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it } from "vitest";
import type {
  DraftView,
  PackComposition,
  PackNode,
  Problem,
  ValidationReport,
} from "../../../api/pack";
import type { Episode, PodcastView } from "../../../api/podcast";
import { expectNoAxeViolations } from "../../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../../test/timeouts";
import { PodcastSelectionProvider, usePodcastSelection } from "../SelectionContext";
import type { SelectedEpisode } from "../selection";
import { PackEditor } from "./PackEditor";

tolerateLoadInAsyncUtils();

const episode = (id: string, extra: Partial<Episode> = {}): Episode => ({
  id,
  title: `Épisode ${id}`,
  description: null,
  pub_date: null,
  duration: 600,
  author: null,
  enclosure_type: "audio/mpeg",
  enclosure_length: 10_000_000,
  has_image: false,
  explicit: false,
  ...extra,
});

function feed(
  sourceId: string,
  title: string,
  episodes: Episode[],
  extra: { copyright?: string | null; has_image?: boolean } = {},
): PodcastView {
  return {
    source_id: sourceId,
    feed: {
      title,
      description: null,
      copyright: extra.copyright ?? null,
      has_image: extra.has_image ?? false,
      episodes,
      warnings: [],
    },
    status: { offline: false, updated_at: 1_790_000_000, error: null },
  };
}

const ep = (source: string, id: string): PackNode => ({
  kind: "episode",
  source_id: source,
  episode_id: id,
});
const cat = (name: string, ...entries: PackNode[]): PackNode => ({
  kind: "category",
  name,
  entries,
});
const composition = (menu: PackNode[], title = ""): PackComposition => ({
  metadata: { title, subtitle: null, age: 0, description: null },
  cover: { kind: "default" },
  menu,
});

function selected(source: string, id: string, title = `Épisode ${id}`): SelectedEpisode {
  return {
    id,
    sourceId: source,
    sourceName: source,
    title,
    bytes: 10_000_000,
    explicit: false,
  };
}

let views: Record<string, PodcastView>;
let draft: DraftView;
let gone: Set<string>;
let calls: { cmd: string; args: Record<string, any> }[];
let saveRevisionAnswer: ((revision: number) => number) | null;

function report(c: PackComposition): ValidationReport {
  const episodes: { source_id: string; episode_id: string }[] = [];
  const walk = (menu: PackNode[]) =>
    menu.forEach((n) => (n.kind === "episode" ? episodes.push(n) : walk(n.entries)));
  walk(c.menu);
  const feeds = new Set(episodes.map((e) => e.source_id)).size;
  const problems: Problem[] = [];
  if (episodes.length === 0) {
    problems.push({
      code: "no_episode",
      message: "aucun épisode",
      path: null,
      episode: null,
      field: null,
    });
  }
  if (feeds > 1 && c.metadata.title.trim() === "") {
    problems.push({
      code: "title_required",
      message: "titre",
      path: null,
      episode: null,
      field: "title",
    });
  }
  const unavailable = episodes.filter((e) => gone.has(e.episode_id));
  for (const e of unavailable) {
    problems.push({
      code: "episode_unavailable",
      message: "indisponible",
      path: [0],
      episode: e,
      field: null,
    });
  }
  return {
    valid: problems.length === 0,
    problems,
    unavailable,
    episodes: episodes.length,
    feeds,
    estimated_bytes: episodes.length * 10_000_000,
    unknown_size: 0,
    metadata: null,
    cover: null,
  };
}

beforeEach(() => {
  views = {};
  draft = { composition: null, revision: 0, notice: null, report: null };
  gone = new Set();
  calls = [];
  saveRevisionAnswer = null;
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    switch (cmd) {
      case "pack_draft_read":
        return draft;
      case "pack_draft_save":
        return {
          saved: true,
          revision: saveRevisionAnswer?.(payload.revision) ?? payload.revision,
        };
      case "pack_validate":
        return report(payload.composition);
      case "podcast_feed":
        return views[payload.sourceId];
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => clearMocks());

function Seed({ items, children }: { items: SelectedEpisode[]; children: ReactNode }) {
  const selection = usePodcastSelection();
  const [done, setDone] = useState(items.length === 0);
  useEffect(() => {
    // Sélection simulée avant l'ouverture de l'éditeur.
    items.forEach((item) => selection.toggle(item));
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setDone(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return done && selection.selected.length === items.length ? <>{children}</> : null;
}

function renderEditor(items: SelectedEpisode[] = []) {
  return render(
    <PodcastSelectionProvider>
      <Seed items={items}>
        <PackEditor saveDelayMs={20} validateDelayMs={20} />
      </Seed>
    </PodcastSelectionProvider>,
  );
}

const count = (cmd: string) => calls.filter((c) => c.cmd === cmd).length;

it("arbre vide : message, problème signalé, création désactivée", async () => {
  renderEditor();
  expect(await screen.findByText("Le pack est vide")).toBeInTheDocument();
  expect(
    await screen.findByText("Le pack ne contient aucun épisode."),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Créer l'histoire" })).toBeDisabled();
  expect(screen.getByText(/pas encore disponible/)).toBeInTheDocument();
});

it("sans brouillon, les épisodes sélectionnés sont rangés à la racine", async () => {
  views.a = feed("a", "Contes", [episode("1"), episode("2")]);
  renderEditor([selected("a", "1"), selected("a", "2")]);
  expect(
    await screen.findByRole("button", { name: "Monter « Épisode 2 »" }),
  ).toBeEnabled();
  expect(screen.getByRole("button", { name: "Monter « Épisode 1 »" })).toBeDisabled();
  expect(screen.queryByText("Épisodes sélectionnés, pas encore placés")).toBeNull();
  // Aucune sauvegarde tant que le parent n'a rien modifié.
  await new Promise((r) => setTimeout(r, 80));
  expect(count("pack_draft_save")).toBe(0);
});

it("monter, descendre et déplacer vers une catégorie, au clavier", async () => {
  views.a = feed("a", "Contes", [episode("1"), episode("2"), episode("3")]);
  draft = {
    ...draft,
    composition: composition([ep("a", "1"), ep("a", "2"), cat("Hiver")]),
  };
  renderEditor();
  fireEvent.click(await screen.findByRole("button", { name: "Descendre « Épisode 1 »" }));
  const root = screen.getByRole("list", { name: "Racine" });
  await waitFor(() => {
    const labels = within(root)
      .getAllByRole("button", { name: /^Monter/ })
      .map((b) => b.getAttribute("aria-label"));
    expect(labels.slice(0, 2)).toEqual(["Monter « Épisode 2 »", "Monter « Épisode 1 »"]);
  });
  // Le focus a suivi l'élément déplacé.
  expect(screen.getByRole("button", { name: "Descendre « Épisode 1 »" })).toHaveFocus();
  expect(screen.getByText("« Épisode 1 » : position 2 sur 3.")).toBeInTheDocument();

  fireEvent.change(screen.getByLabelText("Déplacer « Épisode 1 » vers"), {
    target: { value: "0" },
  });
  const hiver = await screen.findByRole("list", { name: "Racine > Hiver" });
  expect(within(hiver).getByText("Épisode 1")).toBeInTheDocument();
  expect(
    screen.getByText("« Épisode 1 » déplacé vers « Racine > Hiver »."),
  ).toBeInTheDocument();
});

it("limites atteintes : 50 options à la racine, 3 niveaux de menus", async () => {
  const many = Array.from({ length: 50 }, (_, i) => ep("a", `e${i}`));
  views.a = feed(
    "a",
    "Contes",
    many.map((n) => episode((n as any).episode_id)),
  );
  draft = { ...draft, composition: composition(many) };
  renderEditor([selected("a", "extra", "Un de trop")]);
  expect(
    await screen.findByRole("button", { name: "Ajouter une catégorie à la racine" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Ajouter « Un de trop » à la racine" }),
  ).toBeDisabled();
  expect(screen.getByRole("button", { name: "Tout ajouter à la racine" })).toBeDisabled();
});

it("troisième niveau : pas de catégorie au-delà", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = {
    ...draft,
    composition: composition([cat("N1", cat("N2", ep("a", "1")))]),
  };
  renderEditor();
  expect(
    await screen.findByRole("button", {
      name: "Ajouter une catégorie dans « Racine > N1 »",
    }),
  ).toBeEnabled();
  expect(
    screen.getByRole("button", {
      name: "Ajouter une catégorie dans « Racine > N1 > N2 »",
    }),
  ).toBeDisabled();
});

it("doublon refusé : un épisode déjà placé n'est pas reproposé, le doublon du brouillon est signalé", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]) };
  renderEditor([selected("a", "1")]);
  await screen.findByRole("button", { name: "Monter « Épisode 1 »" });
  expect(screen.queryByText("Épisodes sélectionnés, pas encore placés")).toBeNull();
  expect(screen.queryByRole("button", { name: /^Ajouter « Épisode 1 »/ })).toBeNull();
});

it("un épisode sélectionné après coup se place à la racine", async () => {
  views.a = feed("a", "Contes", [episode("1"), episode("2")]);
  draft = { ...draft, composition: composition([ep("a", "1")]) };
  renderEditor([selected("a", "1"), selected("a", "2")]);
  fireEvent.click(
    await screen.findByRole("button", { name: "Ajouter « Épisode 2 » à la racine" }),
  );
  expect(
    await screen.findByRole("button", { name: "Monter « Épisode 2 »" }),
  ).toBeEnabled();
  expect(screen.queryByText("Épisodes sélectionnés, pas encore placés")).toBeNull();
});

it("reprise d'un brouillon avec un épisode indisponible", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  gone.add("2");
  draft = {
    composition: composition([ep("a", "1"), ep("a", "2")]),
    revision: 4,
    notice: null,
    report: null,
  };
  renderEditor();
  expect(await screen.findByText("Indisponible")).toBeInTheDocument();
  expect(
    await screen.findByText(
      "1 épisode n'est plus disponible dans son flux : retirez-le du pack.",
    ),
  ).toBeInTheDocument();
  expect(screen.getByText("Épisode 1")).toBeInTheDocument();
  expect(
    screen.getByText(/Un épisode n'est plus disponible : retirez-le du pack\./),
  ).toBeInTheDocument();
});

it("titre obligatoire en multi-flux, titre du flux proposé en flux unique", async () => {
  views.a = feed("a", "Contes du soir", [episode("1")], { copyright: "© Radio" });
  views.b = feed("b", "Histoires", [episode("2")]);
  draft = { ...draft, composition: composition([ep("a", "1")]) };
  renderEditor();
  expect(
    await screen.findByText(
      /Laissez vide pour reprendre le titre du flux : « Contes du soir »/,
    ),
  ).toBeInTheDocument();
  expect(screen.getByText(/copyright du flux : « © Radio »/)).toBeInTheDocument();
  expect(screen.queryByText(/titre est obligatoire/)).toBeNull();
});

it("titre obligatoire quand le pack réunit plusieurs flux", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  views.b = feed("b", "Histoires", [episode("2")]);
  draft = { ...draft, composition: composition([ep("a", "1"), ep("b", "2")]) };
  renderEditor();
  expect(
    await screen.findByText("Obligatoire : le pack réunit plusieurs flux."),
  ).toBeInTheDocument();
  const error = await screen.findByText(
    "Le titre est obligatoire quand le pack réunit plusieurs flux.",
    { selector: ".fl-field__error" },
  );
  expect(error).toBeInTheDocument();
  expect(screen.getByLabelText("Titre")).toBeInvalid();

  fireEvent.change(screen.getByLabelText("Titre"), { target: { value: "Mon pack" } });
  await waitFor(() =>
    expect(
      screen.queryByText(
        "Le titre est obligatoire quand le pack réunit plusieurs flux.",
        {
          selector: ".fl-field__error",
        },
      ),
    ).toBeNull(),
  );
  expect(await screen.findByText("Le pack est prêt à être créé.")).toBeInTheDocument();
});

it("espace les validations et sauvegarde avec la révision du backend", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = {
    composition: composition([ep("a", "1")]),
    revision: 7,
    notice: null,
    report: null,
  };
  saveRevisionAnswer = (revision) => revision;
  renderEditor();
  const title = await screen.findByLabelText("Titre");
  await waitFor(() => expect(count("pack_validate")).toBe(1));
  for (const value of ["M", "Mo", "Mon", "Mon p", "Mon pa", "Mon pack"]) {
    fireEvent.change(title, { target: { value } });
  }
  await waitFor(() => expect(count("pack_draft_save")).toBe(1));
  await waitFor(() => expect(count("pack_validate")).toBe(2));
  expect(count("pack_draft_save")).toBe(1);
  const save = calls.find((c) => c.cmd === "pack_draft_save")!;
  expect(save.args.revision).toBe(8);
  expect(save.args.composition.metadata.title).toBe("Mon pack");

  fireEvent.change(title, { target: { value: "Autre" } });
  await waitFor(() => expect(count("pack_draft_save")).toBe(2));
  expect(calls.filter((c) => c.cmd === "pack_draft_save")[1]!.args.revision).toBe(9);
});

it("couverture : image d'un flux ou d'un épisode du pack", async () => {
  views.a = feed("a", "Contes", [episode("1", { has_image: true })], { has_image: true });
  draft = { ...draft, composition: composition([ep("a", "1")]) };
  renderEditor();
  const select = await screen.findByLabelText("Image de la couverture");
  await waitFor(() =>
    expect(
      within(select).getByRole("option", { name: "Image de l'épisode « Épisode 1 »" }),
    ).toBeInTheDocument(),
  );
  fireEvent.change(select, { target: { value: "episode:0" } });
  await waitFor(() => expect(count("pack_draft_save")).toBe(1));
  expect(calls.find((c) => c.cmd === "pack_draft_save")!.args.composition.cover).toEqual({
    kind: "episode",
    source_id: "a",
    episode_id: "1",
  });
  expect(
    within(select).getByRole("option", { name: "Image du flux « Contes »" }),
  ).toBeInTheDocument();
});

it("affiche la notice d'un brouillon mis de côté", async () => {
  draft = { ...draft, notice: "Le brouillon de pack précédent était illisible." };
  renderEditor();
  expect(await screen.findByText(/était illisible/)).toBeInTheDocument();
});

it("erreur de lecture du brouillon : message et nouvel essai", async () => {
  let fail = true;
  mockIPC((cmd) => {
    if (cmd === "pack_draft_read") {
      if (fail) throw { kind: "io", message: "disque" };
      return draft;
    }
    if (cmd === "pack_validate") return report(composition([]));
    return undefined;
  });
  renderEditor();
  expect(await screen.findByText("disque")).toBeInTheDocument();
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "Réessayer" }));
  expect(await screen.findByText("Le pack est vide")).toBeInTheDocument();
});

it(
  "aucune violation d'accessibilité",
  async () => {
    views.a = feed("a", "Contes", [episode("1"), episode("2")]);
    draft = {
      ...draft,
      composition: composition([ep("a", "1"), cat("Hiver", ep("a", "2"))]),
    };
    const { container } = renderEditor();
    await screen.findByRole("list", { name: "Racine > Hiver" });
    await screen.findByText("Le pack est prêt à être créé.");
    await expectNoAxeViolations(container);
  },
  SLOW_TEST_MS,
);
