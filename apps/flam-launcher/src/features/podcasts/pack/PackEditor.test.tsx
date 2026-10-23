/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect, useState, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
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

// Fermeture de la fenêtre : le gestionnaire enregistré par l'éditeur est capturé ici.
const closeHandlers = vi.hoisted(() => ({
  current: null as null | (() => Promise<void>),
}));
vi.mock("../../../api/window", () => ({
  onWindowCloseRequested: (handler: () => Promise<void>) => {
    closeHandlers.current = handler;
    return Promise.resolve(() => {
      if (closeHandlers.current === handler) closeHandlers.current = null;
    });
  },
}));

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
  // Choisir une destination ne déplace rien tant qu'on ne valide pas.
  expect(
    within(screen.getByRole("list", { name: "Racine > Hiver" })).queryByText("Épisode 1"),
  ).toBeNull();
  fireEvent.click(
    screen.getByRole("button", { name: "Déplacer « Épisode 1 » vers le menu choisi" }),
  );
  const hiver = await screen.findByRole("list", { name: "Racine > Hiver" });
  expect(within(hiver).getByText("Épisode 1")).toBeInTheDocument();
  expect(
    screen.getByText("« Épisode 1 » déplacé vers « Racine > Hiver »."),
  ).toBeInTheDocument();
});

it(
  "limites atteintes : 50 options à la racine, 3 niveaux de menus",
  async () => {
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
    expect(
      screen.getByRole("button", { name: "Tout ajouter à la racine" }),
    ).toBeDisabled();
  },
  SLOW_TEST_MS,
);

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
  expect(screen.getByText(/Prérempli avec le copyright du flux/)).toBeInTheDocument();
  expect(screen.getByLabelText("Sous-titre")).toHaveValue("© Radio");
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

it("reprise : le rapport lu avec le brouillon évite un premier appel à pack_validate", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  const c = composition([ep("a", "1")]);
  draft = { composition: c, revision: 2, notice: null, report: report(c) };
  renderEditor();
  expect(await screen.findByText("Le pack est prêt à être créé.")).toBeInTheDocument();
  await new Promise((r) => setTimeout(r, 80));
  expect(count("pack_validate")).toBe(0);
});

const saves = () => calls.filter((c) => c.cmd === "pack_draft_save");

/** IPC dont la commande de sauvegarde est fournie par le test. */
function mockWithSave(save: (payload: Record<string, any>) => unknown) {
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    if (cmd === "pack_draft_read") return draft;
    if (cmd === "pack_validate") return report(payload.composition);
    if (cmd === "podcast_feed") return views[payload.sourceId];
    if (cmd === "pack_draft_save") return save(payload);
    throw new Error(`commande inattendue : ${cmd}`);
  });
}

it("sous-titre : prérempli par le copyright du flux unique, effaçable (D16)", async () => {
  views.a = feed("a", "Contes", [episode("1")], { copyright: "© Radio" });
  draft = { ...draft, composition: composition([ep("a", "1")]) };
  renderEditor();
  const subtitle = await screen.findByLabelText("Sous-titre");
  await waitFor(() => expect(subtitle).toHaveValue("© Radio"));
  fireEvent.change(subtitle, { target: { value: "" } });
  expect(subtitle).toHaveValue("");
  await waitFor(() => expect(count("pack_draft_save")).toBe(1));
  expect(saves()[0]!.args.composition.metadata.subtitle).toBe("");
});

it("sauvegarde : une seule à la fois, la dernière composition part ensuite", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 3 };
  let inFlight = 0;
  let maxInFlight = 0;
  let release: (() => void) | null = null;
  mockWithSave(async (payload) => {
    inFlight += 1;
    maxInFlight = Math.max(maxInFlight, inFlight);
    if (saves().length === 1) await new Promise<void>((r) => (release = r));
    inFlight -= 1;
    return { saved: true, revision: payload.revision };
  });
  renderEditor();
  const title = await screen.findByLabelText("Titre");
  fireEvent.change(title, { target: { value: "A" } });
  await waitFor(() => expect(saves().length).toBe(1));
  fireEvent.change(title, { target: { value: "AB" } });
  await new Promise((r) => setTimeout(r, 80));
  expect(saves().length).toBe(1);
  release!();
  await waitFor(() => expect(saves().length).toBe(2));
  expect(maxInFlight).toBe(1);
  expect(saves()[1]!.args.composition.metadata.title).toBe("AB");
  expect(saves()[1]!.args.revision).toBe(5);
});

it("saved == false : recalage sur la révision renvoyée + 1, puis renvoi", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 1 };
  // Le backend est à la révision 9 : la 2 est refusée, la 10 acceptée.
  mockWithSave((payload) =>
    payload.revision <= 9
      ? { saved: false, revision: 9 }
      : { saved: true, revision: payload.revision },
  );
  renderEditor();
  fireEvent.change(await screen.findByLabelText("Titre"), {
    target: { value: "Mon pack" },
  });
  await waitFor(() => expect(saves().length).toBe(2));
  expect(saves().map((c) => c.args.revision)).toEqual([2, 10]);
  expect(saves()[1]!.args.composition.metadata.title).toBe("Mon pack");
  expect(await screen.findByText("Brouillon enregistré.")).toBeInTheDocument();
});

it("échec de sauvegarde : bannière, puis « Réessayer » renvoie la composition", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 4 };
  let fail = true;
  mockWithSave((payload) => {
    if (fail) throw { kind: "io", message: "disque plein" };
    return { saved: true, revision: payload.revision };
  });
  renderEditor();
  fireEvent.change(await screen.findByLabelText("Titre"), {
    target: { value: "Mon pack" },
  });
  const retry = await screen.findByRole("button", { name: "Réessayer" });
  fail = false;
  fireEvent.click(retry);
  await waitFor(() => expect(saves().length).toBe(2));
  expect(saves()[1]!.args.revision).toBe(5);
  expect(saves()[1]!.args.composition.metadata.title).toBe("Mon pack");
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Réessayer" })).toBeNull(),
  );
});

function renderSlowSave(delay = 5000) {
  return render(
    <PodcastSelectionProvider>
      <PackEditor saveDelayMs={delay} validateDelayMs={20} />
    </PodcastSelectionProvider>,
  );
}

it("démontage : la modification en attente est sauvegardée sans attendre le débounce", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 1 };
  const { unmount } = renderSlowSave();
  fireEvent.change(await screen.findByLabelText("Titre"), { target: { value: "Vite" } });
  expect(saves().length).toBe(0);
  unmount();
  await waitFor(() => expect(saves().length).toBe(1));
  expect(saves()[0]!.args.composition.metadata.title).toBe("Vite");
});

it("fermeture de la fenêtre : le débounce est vidé avant la fermeture", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 1 };
  renderSlowSave();
  fireEvent.change(await screen.findByLabelText("Titre"), { target: { value: "Vite" } });
  await waitFor(() => expect(closeHandlers.current).not.toBeNull());
  expect(saves().length).toBe(0);
  await closeHandlers.current!();
  expect(saves().length).toBe(1);
  expect(saves()[0]!.args.composition.metadata.title).toBe("Vite");
});

it("fermeture de la fenêtre : l'attente est bornée si la sauvegarde traîne", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 1 };
  let finish: (() => void) | null = null;
  mockWithSave(
    (payload) =>
      new Promise((resolve) => {
        finish = () => resolve({ saved: true, revision: payload.revision });
      }),
  );
  renderSlowSave();
  fireEvent.change(await screen.findByLabelText("Titre"), { target: { value: "Vite" } });
  await waitFor(() => expect(closeHandlers.current).not.toBeNull());
  const started = Date.now();
  await closeHandlers.current!();
  expect(Date.now() - started).toBeLessThan(2500);
  expect(saves().length).toBe(1);
  finish!();
});

it("quitter puis rouvrir aussitôt : la lecture attend la sauvegarde en cours", async () => {
  views.a = feed("a", "Contes", [episode("1")]);
  draft = { ...draft, composition: composition([ep("a", "1")]), revision: 1 };
  const events: string[] = [];
  mockIPC(async (cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    if (cmd === "pack_draft_read") {
      events.push("read");
      return draft;
    }
    if (cmd === "pack_validate") return report(payload.composition);
    if (cmd === "podcast_feed") return views[payload.sourceId];
    if (cmd === "pack_draft_save") {
      events.push("save-start");
      await new Promise((r) => setTimeout(r, 60));
      events.push("save-end");
      return { saved: true, revision: payload.revision };
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
  const first = renderSlowSave();
  fireEvent.change(await screen.findByLabelText("Titre"), { target: { value: "Vite" } });
  first.unmount();
  renderSlowSave();
  await screen.findByLabelText("Titre");
  expect(events).toEqual(["read", "save-start", "save-end", "read"]);
});

it("retrait d'un élément : le focus passe au voisin ; dernier élément : au parent", async () => {
  views.a = feed("a", "Contes", [episode("1"), episode("2"), episode("3")]);
  draft = {
    ...draft,
    composition: composition([ep("a", "1"), ep("a", "2"), cat("Hiver", ep("a", "3"))]),
  };
  renderEditor();
  fireEvent.click(await screen.findByRole("button", { name: "Retirer « Épisode 1 »" }));
  await waitFor(() =>
    expect(screen.getByLabelText("Déplacer « Épisode 2 » vers")).toHaveFocus(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Retirer « Épisode 3 »" }));
  await waitFor(() =>
    expect(screen.getByLabelText(/Nom de la catégorie n° 2/)).toHaveFocus(),
  );
});

it("déplacement vers un autre menu : le focus suit l'élément déplacé", async () => {
  views.a = feed("a", "Contes", [episode("1"), episode("2")]);
  draft = {
    ...draft,
    composition: composition([ep("a", "1"), ep("a", "2"), cat("Hiver")]),
  };
  renderEditor();
  fireEvent.change(await screen.findByLabelText("Déplacer « Épisode 1 » vers"), {
    target: { value: "0" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Déplacer « Épisode 1 » vers le menu choisi" }),
  );
  await waitFor(() =>
    expect(screen.getByLabelText("Déplacer « Épisode 1 » vers")).toHaveFocus(),
  );
});
