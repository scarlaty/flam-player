/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Source } from "../../api/sources";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../test/timeouts";

tolerateLoadInAsyncUtils();
import { RecoveryProvider } from "../recovery/RecoveryProvider";
import { folderDisplayName, moveItem, SourcesScreen } from "./SourcesScreen";

const pickFolder = vi.hoisted(() => vi.fn());
vi.mock("../../api/dialog", () => ({ pickFolder, pickExecutable: vi.fn() }));

const library: Source = { type: "library", id: "library", name: "Mes histoires" };
const store: Source = {
  type: "catalog",
  id: "telmi-store",
  name: "Telmi Store",
  url: "https://exemple.org/telmi.json",
};
const folder: Source = { type: "folder", id: "f1", name: "Contes", path: "C:\\Contes" };

let sources: Source[];
let calls: { cmd: string; args: Record<string, unknown> }[];
let failures: Record<string, { kind: string; message: string }>;

beforeEach(() => {
  sources = [library, store, folder];
  calls = [];
  failures = {};
  pickFolder.mockReset();
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    const failure = failures[cmd];
    if (failure) throw failure;
    switch (cmd) {
      case "sources_list":
        return { sources, recovery: null };
      case "sources_add": {
        const source = { ...payload.input, id: `new-${sources.length}` } as Source;
        sources = [...sources, source];
        return source;
      }
      case "sources_update": {
        const updated = { ...payload.input, id: payload.id } as Source;
        sources = sources.map((s) => (s.id === payload.id ? updated : s));
        return updated;
      }
      case "sources_remove":
        sources = sources.filter((s) => s.id !== payload.id);
        return null;
      case "sources_reorder": {
        const order = payload.ids as string[];
        sources = [library, ...order.map((id) => sources.find((s) => s.id === id)!)];
        return sources;
      }
      default:
        throw { kind: "unknown", message: cmd };
    }
  });
});

afterEach(() => clearMocks());

async function renderScreen() {
  const view = render(
    <ToastProvider>
      <RecoveryProvider>
        <SourcesScreen />
      </RecoveryProvider>
    </ToastProvider>,
  );
  await screen.findByText("Telmi Store");
  return view;
}

const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);

it("liste les sources ; « Mes histoires » n'a aucune action", async () => {
  await renderScreen();

  const items = within(
    screen.getByRole("list", { name: "Sources enregistrées" }),
  ).getAllByRole("listitem");
  expect(items).toHaveLength(3);
  expect(within(items[0]!).queryByRole("button")).not.toBeInTheDocument();
  expect(
    within(items[0]!).getByText("Toujours en première position"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Monter « Telmi Store »" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Descendre « Contes »" })).toBeDisabled();
});

it("affiche l'erreur de chargement avec un bouton Réessayer", async () => {
  failures.sources_list = { kind: "io", message: "disque illisible" };
  render(
    <ToastProvider>
      <RecoveryProvider>
        <SourcesScreen />
      </RecoveryProvider>
    </ToastProvider>,
  );

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Erreur d'accès aux fichiers");
  expect(alert).toHaveTextContent("disque illisible");

  delete failures.sources_list;
  fireEvent.click(screen.getByRole("button", { name: "Réessayer" }));
  expect(await screen.findByText("Telmi Store")).toBeInTheDocument();
});

it("valide le formulaire de catalogue en direct sans appeler le backend", async () => {
  await renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Ajouter un catalogue" }));
  const dialog = await screen.findByRole("dialog", { name: "Ajouter un catalogue" });

  const name = within(dialog).getByRole("textbox", { name: "Nom" });
  expect(name).toHaveFocus();
  fireEvent.change(name, { target: { value: "  " } });
  expect(within(dialog).getByText("Le nom ne peut pas être vide.")).toBeInTheDocument();

  const url = within(dialog).getByRole("textbox", { name: "Adresse du catalogue" });
  fireEvent.change(url, { target: { value: "ftp://exemple.org" } });
  expect(within(dialog).getByText(/doit commencer par http/)).toBeInTheDocument();
  expect(url).toHaveAttribute("aria-invalid", "true");

  fireEvent.change(url, { target: { value: "https://u:p@exemple.org/c.json" } });
  expect(within(dialog).getByText(/identifiants/)).toBeInTheDocument();

  fireEvent.click(within(dialog).getByRole("button", { name: "Ajouter" }));
  expect(callsOf("sources_add")).toHaveLength(0);
});

it("affiche l'erreur du backend dans le formulaire puis ajoute le catalogue", async () => {
  await renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Ajouter un catalogue" }));
  const dialog = await screen.findByRole("dialog", { name: "Ajouter un catalogue" });
  fireEvent.change(within(dialog).getByRole("textbox", { name: "Nom" }), {
    target: { value: "Mon catalogue" },
  });
  fireEvent.change(
    within(dialog).getByRole("textbox", { name: "Adresse du catalogue" }),
    {
      target: { value: "https://exemple.org/c.json" },
    },
  );

  failures.sources_add = { kind: "validation", message: "URL refusée par le serveur" };
  fireEvent.click(within(dialog).getByRole("button", { name: "Ajouter" }));
  const alert = await within(dialog).findByRole("alert");
  expect(alert).toHaveTextContent("Valeur refusée");
  expect(alert).toHaveTextContent("URL refusée par le serveur");

  delete failures.sources_add;
  fireEvent.click(within(dialog).getByRole("button", { name: "Ajouter" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(callsOf("sources_add").at(-1)?.args.input).toEqual({
    type: "catalog",
    name: "Mon catalogue",
    url: "https://exemple.org/c.json",
  });
  expect(screen.getByText("Mon catalogue")).toBeInTheDocument();
  expect(screen.getByText("Source « Mon catalogue » ajoutée")).toBeInTheDocument();
});

it("ajoute un dossier choisi dans le sélecteur natif", async () => {
  pickFolder.mockResolvedValue("C:\\Users\\moi\\Histoires du soir\\");
  await renderScreen();

  fireEvent.click(screen.getByRole("button", { name: "Ajouter un dossier" }));

  expect(await screen.findByText("Histoires du soir")).toBeInTheDocument();
  expect(callsOf("sources_add")[0]?.args.input).toEqual({
    type: "folder",
    name: "Histoires du soir",
    path: "C:\\Users\\moi\\Histoires du soir\\",
  });
});

it("n'ajoute rien si le sélecteur est annulé, et affiche l'erreur du backend", async () => {
  pickFolder.mockResolvedValueOnce(null);
  await renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Ajouter un dossier" }));
  await waitFor(() => expect(pickFolder).toHaveBeenCalled());
  expect(callsOf("sources_add")).toHaveLength(0);

  pickFolder.mockResolvedValueOnce("C:\\Absent");
  failures.sources_add = { kind: "validation", message: "le dossier n'existe pas" };
  fireEvent.click(screen.getByRole("button", { name: "Ajouter un dossier" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("le dossier n'existe pas");
});

it("renomme une source en conservant son adresse", async () => {
  await renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Renommer « Telmi Store »" }));
  const dialog = await screen.findByRole("dialog", { name: "Renommer la source" });
  const name = within(dialog).getByRole("textbox", { name: "Nom" });
  expect(name).toHaveValue("Telmi Store");

  fireEvent.change(name, { target: { value: "" } });
  expect(within(dialog).getByText("Le nom ne peut pas être vide.")).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole("button", { name: "Renommer" }));
  expect(callsOf("sources_update")).toHaveLength(0);

  fireEvent.change(name, { target: { value: "Boutique" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Renommer" }));

  expect(await screen.findByText("Boutique")).toBeInTheDocument();
  expect(callsOf("sources_update")[0]?.args).toEqual({
    id: "telmi-store",
    input: { type: "catalog", name: "Boutique", url: "https://exemple.org/telmi.json" },
  });
});

it("demande confirmation avant de supprimer, focus sur Annuler", async () => {
  await renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Supprimer « Contes »" }));

  const dialog = await screen.findByRole("alertdialog", {
    name: "Supprimer cette source ?",
  });
  expect(within(dialog).getByRole("button", { name: "Annuler" })).toHaveFocus();
  expect(dialog).toHaveAccessibleDescription(/« Contes » sera retirée/);

  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  expect(callsOf("sources_remove")).toHaveLength(0);

  fireEvent.click(screen.getByRole("button", { name: "Supprimer « Contes »" }));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Supprimer",
    }),
  );
  await waitFor(() => expect(screen.queryByText("Contes")).not.toBeInTheDocument());
  expect(callsOf("sources_remove")[0]?.args).toEqual({ id: "f1" });
});

it("réordonne au clavier avec les boutons et garde le focus", async () => {
  await renderScreen();
  const down = screen.getByRole("button", { name: "Descendre « Telmi Store »" });
  down.focus();
  fireEvent.click(down);

  await waitFor(() => expect(callsOf("sources_reorder")).toHaveLength(1));
  expect(callsOf("sources_reorder")[0]?.args).toEqual({ ids: ["f1", "telmi-store"] });
  const names = screen
    .getAllByRole("listitem")
    .map((li) => li.querySelector(".fl-source-row__name")?.textContent);
  expect(names).toEqual(["Mes histoires", "Contes", "Telmi Store"]);
  // Le bouton « Descendre » est désormais désactivé : le focus passe à « Monter ».
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Monter « Telmi Store »" })).toHaveFocus(),
  );
  expect(
    await screen.findByText("« Telmi Store » : position 2 sur 2"),
  ).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Monter « Telmi Store »" }));
  await waitFor(() => expect(callsOf("sources_reorder")).toHaveLength(2));
  expect(callsOf("sources_reorder")[1]?.args).toEqual({ ids: ["telmi-store", "f1"] });
});

// Course corrigée par #84/#92 : le focus est rendu par un effet de layout. Avec un effet passif
// (`useEffect`), l'effet encore en attente du rendu précédent (ici, la liste qui vient de
// s'afficher) s'exécutait au début du rendu du déplacement et consommait `pendingFocus` avec
// l'ancienne liste : « Descendre » (alors actif) gardait le focus, puis devenait désactivé et le
// focus tombait sur <body>. On clique donc dans la même tâche que l'affichage de la liste.
it(
  "garde le focus du déplacement même si un effet passif est encore en attente",
  async () => {
    const REPEATS = 15;
    for (let i = 0; i < REPEATS; i += 1) {
      sources = [library, store, folder];
      calls = [];
      const view = render(
        <ToastProvider>
          <RecoveryProvider>
            <SourcesScreen />
          </RecoveryProvider>
        </ToastProvider>,
      );
      // Sans `findBy` : on observe l'affichage de la liste au plus tôt, avant tout vidage des effets.
      const find = () =>
        screen.queryByRole("button", { name: "Descendre « Telmi Store »" });
      const down =
        find() ??
        (await new Promise<HTMLElement>((resolve) => {
          const observer = new MutationObserver(() => {
            const button = find();
            if (!button) return;
            observer.disconnect();
            resolve(button);
          });
          observer.observe(document.body, { childList: true, subtree: true });
        }));
      down.focus();
      fireEvent.click(down);

      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "Monter « Telmi Store »" }),
        ).toHaveFocus(),
      );
      view.unmount();
    }
  },
  SLOW_TEST_MS,
);

it("réordonne par glisser-déposer", async () => {
  await renderScreen();
  const rows = screen.getAllByRole("listitem");
  fireEvent.dragStart(rows[2]!);
  fireEvent.dragOver(rows[1]!);
  fireEvent.drop(rows[1]!);

  await waitFor(() => expect(callsOf("sources_reorder")).toHaveLength(1));
  expect(callsOf("sources_reorder")[0]?.args).toEqual({ ids: ["f1", "telmi-store"] });
});

it("rétablit l'ordre et affiche l'erreur si le réordonnancement échoue", async () => {
  await renderScreen();
  failures.sources_reorder = { kind: "validation", message: "ordre refusé" };

  fireEvent.click(screen.getByRole("button", { name: "Descendre « Telmi Store »" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("ordre refusé");
  const names = screen
    .getAllByRole("listitem")
    .map((li) => li.querySelector(".fl-source-row__name")?.textContent);
  expect(names).toEqual(["Mes histoires", "Telmi Store", "Contes"]);
});

it("n'a aucune violation axe, y compris avec un dialogue ouvert", async () => {
  await renderScreen();
  await expectNoAxeViolations(document.body, ["region", "page-has-heading-one"]);

  fireEvent.click(screen.getByRole("button", { name: "Supprimer « Contes »" }));
  await screen.findByRole("alertdialog");
  await expectNoAxeViolations(document.body, ["region", "page-has-heading-one"]);
});

it("propose un nom de dossier et déplace des éléments", () => {
  expect(folderDisplayName("D:\\a\\b\\")).toBe("b");
  expect(folderDisplayName("C:\\")).toBe("C:");
  expect(folderDisplayName("")).toBe("Dossier d'histoires");
  expect(folderDisplayName(`C:\\${"x".repeat(150)}`)).toHaveLength(100);
  expect(moveItem([1, 2, 3], 0, 2)).toEqual([2, 3, 1]);
  expect(moveItem([1, 2, 3], 5, 0)).toEqual([1, 2, 3]);
});
