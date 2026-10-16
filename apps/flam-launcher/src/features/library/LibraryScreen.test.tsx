/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CatalogView } from "../../api/catalog";
import type { FolderScan } from "../../api/folder";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
import { ToastProvider } from "../../components";
import { PAGE_LEVEL_RULES, expectNoAxeViolations } from "../../test/axe";
import { makeEntry } from "../../test/entries";
import { LibraryProvider } from "./LibraryProvider";
import { LibraryScreen } from "./LibraryScreen";
import { LibrarySidebar } from "./LibrarySidebar";
import { PAGE_SIZE } from "./SourceView";
import type { StoryHooks } from "./storyState";
import { slowIt, tolerateLoadInAsyncUtils } from "../../test/timeouts";

tolerateLoadInAsyncUtils();

const library: Source = { type: "library", id: "library", name: "Mes histoires" };
const store: Source = {
  type: "catalog",
  id: "store",
  name: "Catalogue test",
  url: "https://exemple.org/c.json",
};
const folder: Source = {
  type: "folder",
  id: "dossier",
  name: "Contes",
  path: "C:\\Contes",
};

const IMAGE = "data:image/png;base64,AAAA";
const KEPT_IMAGE = "data:image/jpeg;base64,BBBB";

let sources: Source[];
let catalogs: Record<string, CatalogView>;
let refreshed: Record<string, CatalogView>;
let scans: Record<string, FolderScan>;
let failures: Record<string, { kind: string; message: string }>;
let calls: { cmd: string; args: Record<string, any> }[];
let installed: Record<string, any>[];

function catalogView(
  sourceId: string,
  entries: StoryEntry[],
  status: Partial<CatalogView["status"]> = {},
): CatalogView {
  return {
    source_id: sourceId,
    entries,
    warnings: [],
    status: { offline: false, updated_at: 1_700_049_600, error: null, ...status },
  };
}

const storyA = (over: Partial<StoryEntry> = {}) =>
  makeEntry({
    source_id: "store",
    entry_id: "store:a",
    title: "Les trois ours",
    age: 3,
    category: "Contes",
    thumb_url: "https://exemple.org/a.png",
    ...over,
  });
const storyB = (over: Partial<StoryEntry> = {}) =>
  makeEntry({
    source_id: "store",
    entry_id: "store:b",
    title: "Zélie et le robot",
    age: 7,
    category: "Science",
    updated_at: "2025-05-01T00:00:00Z",
    ...over,
  });

beforeEach(() => {
  sources = [library, store, folder];
  catalogs = { store: catalogView("store", [storyA(), storyB()]) };
  refreshed = {};
  scans = { dossier: { entries: [], warnings: [] } };
  failures = {};
  calls = [];
  installed = [];
  window.location.hash = "";
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    const failure = failures[cmd];
    if (failure) throw failure;
    switch (cmd) {
      case "sources_list":
        return { sources, recovery: null };
      case "catalog_entries":
        return catalogs[payload.sourceId];
      case "catalog_refresh": {
        const view = refreshed[payload.sourceId] ?? catalogs[payload.sourceId];
        if (view) catalogs[payload.sourceId] = view;
        return view;
      }
      case "catalog_thumbnail":
        return IMAGE;
      case "folder_scan":
        return scans[payload.sourceId];
      case "folder_thumbnail":
        return IMAGE;
      case "library_list":
        return installed;
      case "library_thumbnail":
        return KEPT_IMAGE;
      default:
        throw { kind: "unknown", message: cmd };
    }
  });
});

afterEach(() => {
  clearMocks();
  window.location.hash = "";
});

function goTo(hash: string) {
  act(() => {
    window.location.hash = hash;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}

function renderLibrary(hooks: StoryHooks = {}) {
  return render(
    <ToastProvider>
      <LibraryProvider>
        <nav aria-label="Sources">
          <LibrarySidebar />
        </nav>
        <main>
          <LibraryScreen {...hooks} />
        </main>
      </LibraryProvider>
    </ToastProvider>,
  );
}

const cardTitles = () =>
  screen.queryAllByRole("heading", { level: 3 }).map((h) => h.textContent);

slowIt(
  "affiche la grille d'un catalogue avec le nombre d'histoires et l'état par défaut",
  async () => {
    goTo("#/library/store");
    renderLibrary();

    expect(
      await screen.findByRole("heading", { level: 2, name: "Catalogue test" }),
    ).toBeInTheDocument();
    await screen.findByText("Les trois ours");
    expect(cardTitles()).toEqual(["Les trois ours", "Zélie et le robot"]);
    expect(screen.getByText("2 histoires")).toBeInTheDocument();
    expect(screen.getAllByText("À télécharger")).toHaveLength(2);
    expect(screen.getByText("Dès 3 ans")).toBeInTheDocument();
    // Le cache est lu sans réseau : pas de rafraîchissement automatique.
    expect(calls.some((c) => c.cmd === "catalog_refresh")).toBe(false);
    await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
  },
);

it("charge les vignettes par la commande de la source", async () => {
  goTo("#/library/store");
  const { container } = renderLibrary();
  await screen.findByText("Les trois ours");
  await waitFor(() => expect(container.querySelectorAll("img")).toHaveLength(1));
  expect(container.querySelector("img")).toHaveAttribute("src", IMAGE);
  const call = calls.find((c) => c.cmd === "catalog_thumbnail");
  expect(call?.args).toEqual({
    sourceId: "store",
    thumbUrl: "https://exemple.org/a.png",
  });
});

it("télécharge un catalogue jamais récupéré", async () => {
  catalogs.store = catalogView("store", [], { updated_at: null });
  refreshed.store = catalogView("store", [storyA()]);
  goTo("#/library/store");
  renderLibrary();
  expect(await screen.findByText("Les trois ours")).toBeInTheDocument();
  expect(calls.some((c) => c.cmd === "catalog_refresh")).toBe(true);
});

slowIt("recherche sans tenir compte des accents et des majuscules", async () => {
  goTo("#/library/store");
  renderLibrary();
  await screen.findByText("Les trois ours");

  fireEvent.change(screen.getByRole("searchbox", { name: "Rechercher une histoire" }), {
    target: { value: "ZELIE" },
  });
  expect(cardTitles()).toEqual(["Zélie et le robot"]);
  expect(await screen.findByText("1 histoire sur 2")).toBeInTheDocument();

  fireEvent.change(screen.getByRole("searchbox", { name: "Rechercher une histoire" }), {
    target: { value: "introuvable" },
  });
  expect(screen.getByRole("heading", { name: "Aucun résultat" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Réinitialiser les filtres" }));
  expect(cardTitles()).toHaveLength(2);
  expect(screen.getByRole("searchbox", { name: "Rechercher une histoire" })).toHaveValue(
    "",
  );
});

it("filtre par âge et par catégorie, et trie", async () => {
  goTo("#/library/store");
  renderLibrary();
  await screen.findByText("Les trois ours");

  const ageSelect = screen.getByLabelText("Âge de l'enfant");
  expect(
    within(ageSelect)
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual(["Tous les âges", "3 ans", "7 ans"]);
  fireEvent.change(ageSelect, { target: { value: "3" } });
  expect(cardTitles()).toEqual(["Les trois ours"]);
  fireEvent.change(ageSelect, { target: { value: "" } });

  fireEvent.change(screen.getByLabelText("Catégorie"), { target: { value: "Science" } });
  expect(cardTitles()).toEqual(["Zélie et le robot"]);
  fireEvent.change(screen.getByLabelText("Catégorie"), { target: { value: "" } });

  fireEvent.change(screen.getByLabelText("Trier par"), { target: { value: "recent" } });
  expect(cardTitles()).toEqual(["Zélie et le robot", "Les trois ours"]);
});

it("propose le tri Popularité seulement si la source publie download_count", async () => {
  goTo("#/library/store");
  const first = renderLibrary();
  await screen.findByText("Les trois ours");
  expect(
    within(screen.getByLabelText("Trier par"))
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual(["Titre (A-Z)", "Plus récentes"]);
  first.unmount();

  catalogs.store = catalogView("store", [
    storyA({ download_count: 3 }),
    storyB({ download_count: 40 }),
  ]);
  renderLibrary();
  await screen.findByText("Les trois ours");
  const sort = screen.getByLabelText("Trier par");
  expect(
    within(sort)
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual(["Titre (A-Z)", "Plus récentes", "Popularité"]);
  fireEvent.change(sort, { target: { value: "popular" } });
  expect(cardTitles()).toEqual(["Zélie et le robot", "Les trois ours"]);
});

slowIt("pagine les grandes sources", async () => {
  const many = Array.from({ length: PAGE_SIZE + 5 }, (_, i) =>
    storyA({ entry_id: `store:${i}`, title: `Histoire ${String(i).padStart(3, "0")}` }),
  );
  catalogs.store = catalogView("store", many);
  goTo("#/library/store");
  renderLibrary({ onOpenStory: () => undefined });
  await screen.findByText("Histoire 000");
  expect(cardTitles()).toHaveLength(PAGE_SIZE);
  fireEvent.click(screen.getByRole("button", { name: "Afficher plus (5 restantes)" }));
  expect(cardTitles()).toHaveLength(PAGE_SIZE + 5);
  expect(screen.queryByRole("button", { name: /Afficher plus/ })).toBeNull();
  // Le bouton a disparu : le focus passe à la première nouvelle carte. Il est posé dans un
  // `useEffect` (passif) : on l'attend au lieu de supposer qu'il est déjà posé au retour du clic.
  const first = screen.getByText(`Histoire ${String(PAGE_SIZE).padStart(3, "0")}`);
  await waitFor(() => {
    expect(first.closest("li")).toContainElement(document.activeElement as HTMLElement);
  });
});

it("n'annonce le nombre de résultats qu'après la frappe", async () => {
  goTo("#/library/store");
  renderLibrary();
  await screen.findByText("Les trois ours");
  expect(screen.getByRole("status")).toHaveTextContent("2 histoires");
  fireEvent.change(screen.getByRole("searchbox", { name: "Rechercher une histoire" }), {
    target: { value: "zel" },
  });
  expect(screen.getByRole("status")).toHaveTextContent("2 histoires");
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("1 histoire"));
});

it("ignore les vignettes qui ne sont pas des images data:", async () => {
  catalogs.store = catalogView("store", [
    storyA({ thumb_url: "data:text/html;base64,AAAA" }),
  ]);
  goTo("#/library/store");
  const { container } = renderLibrary();
  await screen.findByText("Les trois ours");
  expect(container.querySelector("img")).toBeNull();
});

slowIt(
  "signale un catalogue hors ligne et l'indique dans la barre latérale",
  async () => {
    catalogs.store = catalogView("store", [storyA()], { offline: true });
    goTo("#/library/store");
    renderLibrary();
    expect(await screen.findByText("Hors ligne", { selector: "p" })).toBeInTheDocument();
    expect(
      screen.getByText(/dernière vérification, le 15 novembre 2023/),
    ).toBeInTheDocument();
    expect(cardTitles()).toEqual(["Les trois ours"]);
    const side = within(screen.getByRole("navigation", { name: "Sources" }));
    await waitFor(() =>
      expect(
        side.getByRole("button", { name: /Catalogue test.*Hors ligne/ }),
      ).toBeInTheDocument(),
    );
    await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
  },
);

it("signale l'échec du dernier rafraîchissement avec les données en cache", async () => {
  catalogs.store = catalogView("store", [storyA()], {
    error: { kind: "catalog_invalid", message: "JSON invalide" },
  });
  goTo("#/library/store");
  renderLibrary();
  expect(
    await screen.findByText("Le dernier rafraîchissement a échoué"),
  ).toBeInTheDocument();
  expect(screen.getByText("JSON invalide")).toBeInTheDocument();
  const side = within(screen.getByRole("navigation", { name: "Sources" }));
  await waitFor(() =>
    expect(
      side.getByRole("button", { name: /Catalogue test.*Erreur/ }),
    ).toBeInTheDocument(),
  );
});

it("affiche une erreur de chargement avec un bouton Réessayer", async () => {
  failures.catalog_entries = { kind: "network", message: "Pas de réseau" };
  goTo("#/library/store");
  renderLibrary();
  expect(await screen.findByText("Réseau indisponible")).toBeInTheDocument();
  expect(screen.getByText("Pas de réseau")).toBeInTheDocument();

  delete failures.catalog_entries;
  fireEvent.click(screen.getByRole("button", { name: "Réessayer" }));
  expect(await screen.findByText("Les trois ours")).toBeInTheDocument();
  expect(screen.queryByText("Pas de réseau")).toBeNull();
});

it("actualise à la demande et le confirme par une notification", async () => {
  goTo("#/library/store");
  renderLibrary();
  await screen.findByText("Les trois ours");

  refreshed.store = catalogView("store", [
    storyA(),
    storyB(),
    storyB({ entry_id: "store:c", title: "Nouveau" }),
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser" }));
  expect(await screen.findByText("Nouveau")).toBeInTheDocument();
  expect(screen.getByText("« Catalogue test » est à jour")).toBeInTheDocument();
  expect(calls.filter((c) => c.cmd === "catalog_refresh")).toHaveLength(1);
});

it("garde les données et signale l'échec d'une actualisation", async () => {
  goTo("#/library/store");
  renderLibrary();
  await screen.findByText("Les trois ours");

  failures.catalog_refresh = { kind: "network", message: "Délai dépassé" };
  fireEvent.click(screen.getByRole("button", { name: "Actualiser" }));
  await waitFor(() =>
    expect(screen.getAllByText("Délai dépassé").length).toBeGreaterThan(0),
  );
  expect(cardTitles()).toEqual(["Les trois ours", "Zélie et le robot"]);
});

slowIt(
  "affiche un dossier local, ses états par défaut et ses éléments ignorés",
  async () => {
    scans.dossier = {
      entries: [
        makeEntry({
          source_id: "dossier",
          entry_id: "dossier:1",
          title: "Archive Telmi",
          kind: "telmi_zip",
          thumb_url: "dossier:1",
        }),
        makeEntry({
          source_id: "dossier",
          entry_id: "dossier:2",
          title: "Histoire Flam",
          kind: "flam_plain",
        }),
      ],
      warnings: [{ path: "casse.zip", message: "archive illisible" }],
    };
    goTo("#/library/dossier");
    const { container } = renderLibrary();
    expect(await screen.findByText("Archive Telmi")).toBeInTheDocument();
    expect(screen.getByText("À convertir")).toBeInTheDocument();
    expect(screen.getByText("À installer")).toBeInTheDocument();
    expect(screen.getByText("1 élément ignoré")).toBeInTheDocument();
    expect(screen.getByText("casse.zip : archive illisible")).toBeInTheDocument();
    await waitFor(() => expect(container.querySelectorAll("img")).toHaveLength(1));
    const call = calls.find((c) => c.cmd === "folder_thumbnail");
    expect(call?.args).toEqual({ sourceId: "dossier", entryId: "dossier:1" });
    await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
  },
);

it("signale un dossier introuvable", async () => {
  failures.folder_scan = { kind: "folder_unavailable", message: "Disque débranché" };
  goTo("#/library/dossier");
  renderLibrary();
  expect(await screen.findByText("Dossier introuvable")).toBeInTheDocument();
  const side = within(screen.getByRole("navigation", { name: "Sources" }));
  await waitFor(() =>
    expect(side.getByRole("button", { name: /Contes.*Erreur/ })).toBeInTheDocument(),
  );
});

slowIt("gère les listes vides de chaque type de source", async () => {
  catalogs.store = catalogView("store", []);
  goTo("#/library/store");
  renderLibrary();
  expect(await screen.findByText("Ce catalogue est vide")).toBeInTheDocument();
  expect(screen.queryByRole("searchbox")).toBeNull();

  goTo("#/library/dossier");
  expect(await screen.findByText("Aucune histoire dans ce dossier")).toBeInTheDocument();

  goTo("#/library/library");
  expect(await screen.findByText("Aucune histoire installée")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Gérer les sources" })).toHaveAttribute(
    "href",
    "#/sources",
  );
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});

it("ouvre « Mes histoires » par défaut et liste les sources dans l'ordre", async () => {
  goTo("#/library");
  renderLibrary();
  expect(
    await screen.findByRole("heading", { level: 2, name: "Mes histoires" }),
  ).toBeInTheDocument();
  const side = within(screen.getByRole("navigation", { name: "Sources" }));
  const buttons = side.getAllByRole("button");
  expect(buttons.map((b) => b.textContent)).toEqual([
    expect.stringContaining("Mes histoires"),
    expect.stringContaining("Catalogue test"),
    expect.stringContaining("Contes"),
  ]);
  expect(buttons[0]).toHaveAttribute("aria-current", "page");

  fireEvent.click(buttons[1]!);
  expect(window.location.hash).toBe("#/library/store");
});

it("signale une source inconnue", async () => {
  goTo("#/library/disparue");
  renderLibrary();
  expect(await screen.findByText("Source introuvable")).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "Aller à la première source" }),
  ).toHaveAttribute("href", "#/library");
});

it("signale l'échec du chargement des sources", async () => {
  failures.sources_list = { kind: "io", message: "Fichier verrouillé" };
  renderLibrary();
  expect(await screen.findByText("Fichier verrouillé")).toBeInTheDocument();
  failures = {};
  fireEvent.click(screen.getByRole("button", { name: "Réessayer" }));
  expect(
    await screen.findByRole("heading", { name: "Mes histoires", level: 2 }),
  ).toBeInTheDocument();
});

slowIt("laisse les fonctionnalités voisines décorer les cartes", async () => {
  const opened: string[] = [];
  goTo("#/library/store");
  renderLibrary({
    resolveState: (entry) =>
      entry.title === "Zélie et le robot" ? "incompatible" : undefined,
    progressFor: (entry) =>
      entry.title === "Les trois ours" ? { value: 40, text: "Conversion…" } : undefined,
    renderActions: (entry) => <button type="button">Jouer {entry.title}</button>,
    onOpenStory: (entry) => opened.push(entry.entry_id),
    selectedEntryId: "store:b",
  });
  await screen.findByText("Les trois ours");

  expect(screen.getByText("En cours")).toBeInTheDocument();
  expect(screen.getByText("Conversion…")).toBeInTheDocument();
  expect(screen.getByText("Incompatible")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Jouer Zélie et le robot" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Zélie et le robot" })).toHaveAttribute(
    "aria-current",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Les trois ours" }));
  expect(opened).toEqual(["store:a"]);
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});

it("ignore la réponse d'une source quittée avant la fin du chargement", async () => {
  let release: (view: CatalogView) => void = () => undefined;
  const slow = new Promise<CatalogView>((resolve) => (release = resolve));
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    switch (cmd) {
      case "sources_list":
        return { sources, recovery: null };
      case "catalog_entries":
        return payload.sourceId === "store" ? slow : catalogs.store;
      case "folder_scan":
        return { entries: [makeEntry({ title: "Du dossier" })], warnings: [] };
      default:
        throw { kind: "unknown", message: cmd };
    }
  });
  goTo("#/library/store");
  renderLibrary();
  goTo("#/library/dossier");
  expect(await screen.findByText("Du dossier")).toBeInTheDocument();
  await act(async () => release(catalogView("store", [storyA()])));
  expect(screen.queryByText("Les trois ours")).toBeNull();
  expect(screen.getByText("Du dossier")).toBeInTheDocument();
});

it("ne demande la vignette qu'une fois la carte proche de l'écran", async () => {
  const observers: { callback: IntersectionObserverCallback }[] = [];
  class FakeObserver {
    constructor(callback: IntersectionObserverCallback) {
      observers.push({ callback });
    }
    observe() {}
    disconnect() {}
  }
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  try {
    goTo("#/library/store");
    const { container } = renderLibrary();
    await screen.findByText("Les trois ours");
    expect(calls.some((c) => c.cmd === "catalog_thumbnail")).toBe(false);

    act(() => {
      for (const { callback } of observers) {
        callback(
          [{ isIntersecting: true } as IntersectionObserverEntry],
          {} as IntersectionObserver,
        );
      }
    });
    await waitFor(() => expect(container.querySelectorAll("img")).toHaveLength(1));
  } finally {
    vi.unstubAllGlobals();
  }
});

/** Histoire installée depuis le catalogue `store` (#80). */
function installStoryA() {
  const source = storyA();
  installed = [
    {
      uuid: source.uuid,
      version: source.version,
      title: source.title,
      file: `${source.uuid}-1.plain.pk`,
      size: 1,
      installed_at: 1,
      converter: null,
      source,
      thumb: "a-1.1.jpg",
    },
  ];
  return source;
}

it("« Mes histoires » affiche la vignette conservée par la bibliothèque", async () => {
  const source = installStoryA();
  goTo("#/library");
  const { container } = renderLibrary();
  await screen.findByText("Les trois ours");
  await waitFor(() =>
    expect(container.querySelector("img")).toHaveAttribute("src", KEPT_IMAGE),
  );
  expect(calls.find((c) => c.cmd === "library_thumbnail")?.args).toEqual({
    uuid: source.uuid,
  });
  expect(calls.some((c) => c.cmd === "catalog_thumbnail")).toBe(false);
});

it("sans vignette conservée, retombe sur la source d'origine", async () => {
  installStoryA();
  failures = { library_thumbnail: { kind: "not_found", message: "pas de vignette" } };
  goTo("#/library");
  const { container } = renderLibrary();
  await screen.findByText("Les trois ours");
  await waitFor(() =>
    expect(container.querySelector("img")).toHaveAttribute("src", IMAGE),
  );
});

it("sans aucune vignette, garde l'illustration par défaut", async () => {
  installStoryA();
  failures = {
    library_thumbnail: { kind: "not_found", message: "pas de vignette" },
    catalog_thumbnail: { kind: "network", message: "hors ligne" },
  };
  goTo("#/library");
  const { container } = renderLibrary();
  await screen.findByText("Les trois ours");
  await waitFor(() =>
    expect(calls.some((c) => c.cmd === "catalog_thumbnail")).toBe(true),
  );
  expect(container.querySelector("img")).toBeNull();
});
