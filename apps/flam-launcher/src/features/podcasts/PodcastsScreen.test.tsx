/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Episode, Feed, PodcastView } from "../../api/podcast";
import type { Source } from "../../api/sources";
import { ToastProvider } from "../../components";
import { routeFromHash } from "../../router";
import { expectNoAxeViolations } from "../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { PodcastsScreen } from "./PodcastsScreen";
import { PodcastSelectionProvider, usePodcastSelection } from "./SelectionContext";

tolerateLoadInAsyncUtils();

const library: Source = { type: "library", id: "library", name: "Mes histoires" };

function episode(id: string, extra: Partial<Episode> = {}): Episode {
  return {
    id,
    title: `Épisode ${id}`,
    description: null,
    pub_date: "2026-03-12T10:00:00Z",
    duration: 600,
    author: null,
    enclosure_type: "audio/mpeg",
    enclosure_length: 10_000_000,
    has_image: false,
    explicit: false,
    ...extra,
  };
}

function view(sourceId: string, episodes: Episode[], title = "Flux"): PodcastView {
  const feed: Feed = {
    title,
    description: null,
    copyright: null,
    has_image: false,
    episodes,
    warnings: [],
  };
  return {
    source_id: sourceId,
    feed,
    status: { offline: false, updated_at: 1_790_000_000, error: null },
  };
}

function podcast(id: string, name: string): Source {
  return {
    type: "podcast",
    id,
    name,
    url: `https://exemple.org/${id}.xml?token=secret`,
    feed_hint: "https://exemple.org/…",
  };
}

let sources: Source[];
let views: Record<string, PodcastView>;
let calls: { cmd: string; args: Record<string, any> }[];
let failures: Record<string, { kind: string; message: string }>;

beforeEach(() => {
  sources = [library];
  views = {};
  calls = [];
  failures = {};
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    const failure = failures[cmd];
    if (failure) throw failure;
    switch (cmd) {
      case "sources_list":
        return { sources, recovery: null };
      case "podcast_feed":
      case "podcast_refresh":
        return views[payload.sourceId];
      case "podcast_add": {
        const source = podcast("new", payload.name ?? "Nouveau");
        views.new = view("new", [episode("n1")]);
        sources = [...sources, source];
        return { source, view: views.new };
      }
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => clearMocks());

function Probe() {
  const { totals } = usePodcastSelection();
  return <output data-testid="probe">{totals.count}</output>;
}

function renderScreen() {
  return render(
    <ToastProvider>
      <PodcastSelectionProvider>
        <PodcastsScreen />
        <Probe />
      </PodcastSelectionProvider>
    </ToastProvider>,
  );
}

function check(name: string | RegExp) {
  fireEvent.click(screen.getByRole("checkbox", { name }));
}

async function fillAndSubmit(url: string, name?: string) {
  await screen.findByText("Aucun flux podcast");
  fireEvent.change(screen.getByLabelText(/Adresse du flux/), { target: { value: url } });
  if (name !== undefined) {
    fireEvent.change(screen.getByLabelText(/Nom \(facultatif\)/), {
      target: { value: name },
    });
  }
  fireEvent.click(screen.getByRole("button", { name: "Ajouter le flux" }));
}

it("état vide : aucun flux, formulaire d'ajout et mention d'usage", async () => {
  renderScreen();
  expect(await screen.findByText("Aucun flux podcast")).toBeInTheDocument();
  expect(screen.getByLabelText(/Adresse du flux/)).toBeInTheDocument();
  expect(screen.getByText(/publicité comprise/)).toBeInTheDocument();
});

it("liste les épisodes d'un flux et affiche l'URL en clair (D11)", async () => {
  sources = [library, podcast("a", "Contes")];
  views.a = view("a", [episode("1"), episode("2")], "Contes du soir");
  renderScreen();
  expect(
    await screen.findByRole("heading", { name: "Épisodes de « Contes du soir »" }),
  ).toBeInTheDocument();
  expect(await screen.findByRole("checkbox", { name: "Épisode 1" })).not.toBeChecked();
  expect(screen.getByText(/exemple\.org\/a\.xml\?token=secret/)).toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: "Épisode 2" })).toBeInTheDocument();
});

it("un épisode explicite est signalé et décoché ; on peut le cocher volontairement", async () => {
  sources = [library, podcast("a", "Contes")];
  views.a = view("a", [episode("1", { explicit: true })]);
  renderScreen();
  const box = await screen.findByRole("checkbox", { name: "Épisode 1" });
  expect(box).not.toBeChecked();
  expect(box).toHaveAccessibleDescription(/Explicite/);
  fireEvent.click(box);
  expect(box).toBeChecked();
  expect(screen.getByTestId("probe")).toHaveTextContent("1");
});

it("la sélection est annoncée et se vide d'un clic", async () => {
  sources = [library, podcast("a", "Contes")];
  views.a = view("a", [episode("1"), episode("2")]);
  renderScreen();
  await screen.findByRole("checkbox", { name: "Épisode 1" });
  check("Épisode 1");
  const status = screen.getByRole("status", { name: "Sélection" });
  expect(status).toHaveTextContent("1 épisode sélectionné sur 50 au plus");
  fireEvent.click(screen.getByRole("button", { name: "Tout désélectionner" }));
  expect(status).toHaveTextContent("Aucun épisode sélectionné.");
});

it("deux flux : épisodes cumulés ; un même identifiant dans deux flux reste deux épisodes", async () => {
  sources = [library, podcast("a", "A"), podcast("b", "B")];
  views.a = view("a", [episode("1"), episode("shared")], "Flux A");
  views.b = view("b", [episode("2"), episode("shared")], "Flux B");
  renderScreen();
  await screen.findByRole("checkbox", { name: "Épisode 1" });
  check("Épisode 1");
  check("Épisode shared");
  fireEvent.click(screen.getByRole("button", { name: "B" }));
  await screen.findByRole("heading", { name: /Flux B/ });
  await screen.findByRole("checkbox", { name: "Épisode 2" });
  check("Épisode 2");
  expect(screen.getByRole("checkbox", { name: "Épisode shared" })).not.toBeChecked();
  check("Épisode shared");
  expect(screen.getByTestId("probe")).toHaveTextContent("4");
});

it(
  "plafond de 50 épisodes : les autres cases sont désactivées et expliquées",
  async () => {
    const many = Array.from({ length: 51 }, (_, i) =>
      episode(String(i), { title: `Ep ${i}`, enclosure_length: 1000 }),
    );
    sources = [library, podcast("a", "A")];
    views.a = view("a", many);
    renderScreen();
    await screen.findByRole("checkbox", { name: "Ep 0" });
    for (const box of screen.getAllByRole("checkbox").slice(0, 50)) fireEvent.click(box);
    expect(screen.getByTestId("probe")).toHaveTextContent("50");
    fireEvent.click(screen.getByRole("button", { name: /Afficher plus/ }));
    const last = await screen.findByRole("checkbox", { name: "Ep 50" });
    expect(last).toHaveAttribute("aria-disabled", "true");
    expect(last).toHaveAccessibleDescription(/50 épisodes par pack au plus/);
    // Un seul message de plafond pour toutes les cases refusées.
    expect(screen.getAllByText(/50 épisodes par pack au plus/)).toHaveLength(1);
    // Case refusée : atteignable au clavier, et un clic ne change rien.
    last.focus();
    expect(last).toHaveFocus();
    fireEvent.click(last);
    expect(last).not.toBeChecked();
    expect(screen.getByTestId("probe")).toHaveTextContent("50");
  },
  SLOW_TEST_MS,
);

it("plafond par épisode et plafond du pack", async () => {
  sources = [library, podcast("a", "A")];
  const big = (id: string, length: number) =>
    episode(id, { title: id, enclosure_length: length });
  views.a = view("a", [
    big("Énorme", 250_000_000),
    ...["B1", "B2", "B3", "B4", "B5", "B6"].map((id) => big(id, 200_000_000)),
  ]);
  renderScreen();
  const huge = await screen.findByRole("checkbox", { name: "Énorme" });
  expect(huge).toHaveAttribute("aria-disabled", "true");
  expect(huge).toHaveAccessibleDescription(/Épisode trop volumineux/);
  for (const name of ["B1", "B2", "B3", "B4", "B5"]) check(name);
  const over = screen.getByRole("checkbox", { name: "B6" });
  expect(over).toHaveAttribute("aria-disabled", "true");
  expect(over).toHaveAccessibleDescription(/Plafond atteint/);
});

it("ajoute un flux et affiche ses épisodes", async () => {
  renderScreen();
  await fillAndSubmit("https://exemple.org/new.xml", "Mon flux");
  expect(await screen.findByRole("checkbox", { name: "Épisode n1" })).toBeInTheDocument();
  expect(calls.find((c) => c.cmd === "podcast_add")?.args).toEqual({
    url: "https://exemple.org/new.xml",
    name: "Mon flux",
  });
  // La vue rapportée par l'ajout suffit : ni lecture du cache ni seconde actualisation.
  expect(calls.some((c) => c.cmd === "podcast_feed" || c.cmd === "podcast_refresh")).toBe(
    false,
  );
  // Le focus va au titre de la liste, et l'ajout est annoncé par un toast.
  expect(screen.getByRole("heading", { name: /Épisodes de « Flux »/ })).toHaveFocus();
  expect(screen.getByText("Flux « Mon flux » ajouté.")).toBeInTheDocument();
});

it("refuse une adresse invalide sans appeler le backend", async () => {
  renderScreen();
  await fillAndSubmit("pas une adresse");
  expect(await screen.findByText(/doit commencer par http/)).toBeInTheDocument();
  expect(calls.some((c) => c.cmd === "podcast_add")).toBe(false);
});

it.each([
  ["feed_invalid", "flux invalide : XML mal formé", "Flux invalide"],
  ["feed_invalid", "aucun épisode audio exploitable dans le flux", "Aucun épisode audio"],
  ["feed_invalid", "flux trop volumineux : 9 octets (maximum 5)", "Flux trop grand"],
  ["network", "flux inaccessible : connexion impossible", "Flux injoignable"],
  [
    "network",
    "téléchargement refusé : adresse privée, locale ou non publique",
    "Adresse refusée",
  ],
  [
    "network",
    "flux inaccessible : redirection refusée ou trop longue",
    "Flux injoignable",
  ],
  ["validation", "ce flux podcast est déjà dans vos sources", "Flux déjà ajouté"],
])("erreur d'ajout %s : %s", async (kind, message, title) => {
  failures.podcast_add = { kind, message };
  renderScreen();
  await fillAndSubmit("https://exemple.org/x.xml");
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText(title)).toBeInTheDocument();
  expect(within(alert).getByText(message)).toBeInTheDocument();
  expect(alert).not.toHaveTextContent("exemple.org");
});

it("hors ligne : la version enregistrée est affichée avec un avis", async () => {
  sources = [library, podcast("a", "A")];
  views.a = {
    ...view("a", [episode("1")]),
    status: { offline: true, updated_at: 1_790_000_000, error: null },
  };
  renderScreen();
  expect(await screen.findByRole("checkbox", { name: "Épisode 1" })).toBeInTheDocument();
  expect(await screen.findByText(/Hors ligne : la version du flux/)).toBeInTheDocument();
});

it("flux devenu illisible : épisodes conservés et avertissement avec la cause", async () => {
  sources = [library, podcast("a", "A")];
  views.a = {
    ...view("a", [episode("1")]),
    status: {
      offline: false,
      updated_at: 1_790_000_000,
      error: { kind: "feed_invalid", message: "flux invalide : x" },
    },
  };
  renderScreen();
  expect(await screen.findByRole("checkbox", { name: "Épisode 1" })).toBeInTheDocument();
  expect(await screen.findByText(/n'a pas pu être actualisé/)).toBeInTheDocument();
  expect(screen.getByText("flux invalide : x")).toBeInTheDocument();
});

it("flux sans cache et injoignable : erreur affichée, aucune liste", async () => {
  sources = [library, podcast("a", "A")];
  views.a = { ...view("a", []), feed: null };
  failures.podcast_refresh = {
    kind: "network",
    message: "flux inaccessible : délai dépassé",
  };
  renderScreen();
  expect(
    await screen.findByText("flux inaccessible : délai dépassé"),
  ).toBeInTheDocument();
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
});

it("flux sans épisode : état vide", async () => {
  sources = [library, podcast("a", "A")];
  views.a = view("a", []);
  renderScreen();
  expect(await screen.findByText("Aucun épisode")).toBeInTheDocument();
});

it("les cases sont des champs natifs atteignables au clavier", async () => {
  sources = [library, podcast("a", "A")];
  views.a = view("a", [episode("1")]);
  renderScreen();
  const box = await screen.findByRole("checkbox", { name: "Épisode 1" });
  box.focus();
  expect(box).toHaveFocus();
  expect(box).toHaveAttribute("type", "checkbox");
  expect(box).not.toHaveAttribute("tabindex", "-1");
  await waitFor(() => expect(box).toBeEnabled());
});

it(
  "ne présente aucune violation d'accessibilité détectable",
  async () => {
    sources = [library, podcast("a", "A")];
    views.a = view("a", [episode("1", { explicit: true }), episode("2")]);
    const { container } = renderScreen();
    await screen.findByRole("checkbox", { name: "Épisode 1" });
    await expectNoAxeViolations(container, [
      "region",
      "landmark-one-main",
      "page-has-heading-one",
    ]);
  },
  SLOW_TEST_MS,
);

it("« Afficher plus » : le focus passe au premier épisode ajouté", async () => {
  const many = Array.from({ length: 120 }, (_, i) =>
    episode(String(i), { title: `Ep ${i}`, enclosure_length: 1000 }),
  );
  sources = [library, podcast("a", "A")];
  views.a = view("a", many);
  renderScreen();
  await screen.findByRole("checkbox", { name: "Ep 0" });
  expect(screen.getAllByRole("checkbox")).toHaveLength(50);
  const more = screen.getByRole("button", { name: /Afficher plus \(70 restants\)/ });
  more.focus();
  fireEvent.click(more);
  await waitFor(() =>
    expect(screen.getByRole("checkbox", { name: "Ep 50" })).toHaveFocus(),
  );
  expect(screen.getAllByRole("checkbox")).toHaveLength(100);
  fireEvent.click(screen.getByRole("button", { name: /Afficher plus \(20 restants\)/ }));
  await waitFor(() =>
    expect(screen.getByRole("checkbox", { name: "Ep 100" })).toHaveFocus(),
  );
  expect(screen.getAllByRole("checkbox")).toHaveLength(120);
  expect(screen.queryByRole("button", { name: /Afficher plus/ })).not.toBeInTheDocument();
});

it("changer de flux remet la pagination à zéro", async () => {
  const many = Array.from({ length: 60 }, (_, i) =>
    episode(`a${i}`, { title: `A ${i}` }),
  );
  sources = [library, podcast("a", "A"), podcast("b", "B")];
  views.a = view("a", many, "Flux A");
  views.b = view(
    "b",
    many.map((e) => ({ ...e, id: `b${e.id}`, title: `B ${e.title}` })),
    "Flux B",
  );
  renderScreen();
  await screen.findByRole("checkbox", { name: "A 0" });
  fireEvent.click(screen.getByRole("button", { name: /Afficher plus/ }));
  await screen.findByRole("checkbox", { name: "A 59" });
  fireEvent.click(screen.getByRole("button", { name: "B" }));
  await screen.findByRole("heading", { name: /Flux B/ });
  await screen.findByRole("checkbox", { name: "B A 0" });
  expect(screen.getAllByRole("checkbox")).toHaveLength(50);
  expect(
    screen.getByRole("button", { name: /Afficher plus \(10 restants\)/ }),
  ).toBeVisible();
});

it("recliquer le flux déjà choisi ne déplace pas le focus", async () => {
  sources = [library, podcast("a", "A")];
  views.a = view("a", [episode("1")]);
  renderScreen();
  const box = await screen.findByRole("checkbox", { name: "Épisode 1" });
  box.focus();
  fireEvent.click(screen.getByRole("button", { name: "A" }));
  expect(box).toHaveFocus();
});

it("liste des flux en échec : message et « Réessayer »", async () => {
  failures.sources_list = { kind: "io", message: "lecture impossible" };
  renderScreen();
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText(/lecture impossible/)).toBeInTheDocument();
  delete failures.sources_list;
  fireEvent.click(within(alert).getByRole("button", { name: "Réessayer" }));
  expect(await screen.findByText("Aucun flux podcast")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("actualisation en échec avec un cache : épisodes conservés et avertissement", async () => {
  sources = [library, podcast("a", "A")];
  views.a = view("a", [episode("1")]);
  failures.podcast_refresh = { kind: "network", message: "flux inaccessible : délai" };
  renderScreen();
  expect(await screen.findByRole("checkbox", { name: "Épisode 1" })).toBeInTheDocument();
  expect(await screen.findByText(/n'a pas pu être actualisé/)).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("la sélection multi-flux reste visible quand le flux choisi est en erreur", async () => {
  sources = [library, podcast("a", "A"), podcast("b", "B")];
  views.a = view("a", [episode("1")], "Flux A");
  views.b = { ...view("b", []), feed: null };
  renderScreen();
  await screen.findByRole("checkbox", { name: "Épisode 1" });
  check("Épisode 1");
  failures.podcast_refresh = { kind: "network", message: "flux inaccessible : délai" };
  fireEvent.click(screen.getByRole("button", { name: "B" }));
  expect(await screen.findByText("flux inaccessible : délai")).toBeInTheDocument();
  expect(screen.getByRole("status", { name: "Sélection" })).toHaveTextContent(
    "1 épisode sélectionné",
  );
});

it("adresse invalide : le focus va au premier champ invalide", async () => {
  renderScreen();
  await fillAndSubmit("pas une adresse");
  await screen.findByText(/doit commencer par http/);
  expect(screen.getByLabelText(/Adresse du flux/)).toHaveFocus();
});

it("D11 : l'adresse du flux n'est jamais journalisée", async () => {
  const spies = (["log", "info", "warn", "error", "debug"] as const).map((method) =>
    vi.spyOn(console, method).mockImplementation(() => undefined),
  );
  try {
    sources = [library, podcast("a", "A")];
    views.a = view("a", [episode("1")]);
    renderScreen();
    await screen.findByRole("checkbox", { name: "Épisode 1" });
    failures.podcast_add = { kind: "network", message: "flux inaccessible" };
    fireEvent.change(screen.getByLabelText(/Adresse du flux/), {
      target: { value: "https://exemple.org/x.xml?token=secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Ajouter le flux" }));
    await screen.findByRole("alert");
    for (const spy of spies) expect(spy).not.toHaveBeenCalled();
  } finally {
    for (const spy of spies) spy.mockRestore();
  }
});

it("routeFromHash reconnaît #/podcasts", () => {
  expect(routeFromHash("#/podcasts")).toBe("podcasts");
});
