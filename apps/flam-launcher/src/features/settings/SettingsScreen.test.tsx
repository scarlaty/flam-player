/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SettingsView } from "../../api/settings";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { RecoveryProvider } from "../recovery/RecoveryProvider";
import { SettingsScreen } from "./SettingsScreen";

const pickExecutable = vi.hoisted(() => vi.fn());
const pickFolder = vi.hoisted(() => vi.fn());
vi.mock("../../api/dialog", () => ({ pickExecutable, pickFolder }));

function makeView(overrides: Partial<SettingsView> = {}): SettingsView {
  return {
    settings: {
      schema_version: 1,
      player_path: null,
      python_path: null,
      cache_dir: null,
      default_catalog_url: "https://exemple.org/c.json",
      theme: "system",
      max_downloads: 2,
    },
    resolved: {
      player_path: "C:\\app\\flam-player.exe",
      python_path: null,
      cache_dir: "C:\\cache",
    },
    issues: [],
    recovery: null,
    ...overrides,
  };
}

let view: SettingsView;
let usageBytes: number;
let calls: { cmd: string; args: Record<string, any> }[];
let failures: Record<string, { kind: string; message: string }>;
// Réponses différées de `settings_set` (résolues/rejetées par le test, dans l'ordre des appels).
let pendingSets: { resolve: () => void; reject: () => void }[] | null;

beforeEach(() => {
  view = makeView();
  usageBytes = 2048;
  calls = [];
  failures = {};
  pendingSets = null;
  pickExecutable.mockReset();
  pickFolder.mockReset();
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, any>;
    calls.push({ cmd, args: payload });
    const failure = failures[cmd];
    if (failure) throw failure;
    switch (cmd) {
      case "settings_get":
        return view;
      case "settings_set":
        if (pendingSets) {
          const settings = payload.settings;
          return new Promise((resolve, reject) => {
            pendingSets?.push({
              resolve: () => {
                view = { ...view, settings };
                resolve(view);
              },
              reject: () => reject({ kind: "io", message: "disque plein" }),
            });
          });
        }
        view = { ...view, settings: payload.settings };
        return view;
      case "app_info":
        return { name: "flam-launcher", version: "0.1.0" };
      case "cache_usage":
        return {
          path: "C:\\cache",
          bytes: usageBytes,
          subdirs: ["archives", "catalogs", "library", "thumbs"],
        };
      case "cache_clear": {
        const freed = usageBytes;
        usageBytes = 0;
        return { freed_bytes: freed };
      }
      case "config_ack_recovery":
        return null;
      default:
        throw { kind: "unknown", message: cmd };
    }
  });
});

afterEach(() => {
  clearMocks();
  document.documentElement.removeAttribute("data-theme");
});

async function renderScreen() {
  const result = render(
    <ToastProvider>
      <RecoveryProvider>
        <SettingsScreen />
      </RecoveryProvider>
    </ToastProvider>,
  );
  await screen.findByRole("textbox", { name: "Chemin de flam-player" });
  return result;
}

const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);

it("affiche les valeurs utilisées, les problèmes et la section À propos", async () => {
  view = makeView({
    issues: [
      { field: "python_path", kind: "store_alias", message: "alias du Store ignoré" },
    ],
  });
  await renderScreen();

  expect(screen.getByText("Utilisé : C:\\app\\flam-player.exe")).toBeInTheDocument();
  expect(screen.getAllByText(/Aucun fichier trouvé/)).toHaveLength(1);
  expect(screen.getByText("alias du Store ignoré")).toBeInTheDocument();
  expect(screen.getByText(/embarque son propre Python/)).toBeInTheDocument();
  expect(await screen.findByText("flam-launcher — version 0.1.0")).toBeInTheDocument();
  expect(screen.getByText("Licence MIT.")).toBeInTheDocument();
  expect(screen.getByText(/Telmi Store et de TelmiSync/)).toBeInTheDocument();
  expect(screen.getByText("2 Ko occupés dans C:\\cache")).toBeInTheDocument();
});

it("valide les chemins en direct et n'enregistre rien tant qu'ils sont invalides", async () => {
  await renderScreen();
  const player = screen.getByRole("textbox", { name: "Chemin de flam-player" });

  fireEvent.change(player, { target: { value: "C:\\app\\run.bat" } });
  expect(screen.getByText("Le fichier doit se terminer par .exe.")).toBeInTheDocument();
  fireEvent.change(player, { target: { value: "flam-player.exe" } });
  expect(screen.getByText(/chemin complet/)).toBeInTheDocument();
  fireEvent.change(player, { target: { value: "\\\\srv\\p\\flam-player.exe" } });
  expect(screen.getByText(/chemins réseau/)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
  expect(callsOf("settings_set")).toHaveLength(0);
});

it("enregistre l'objet complet avec les chemins choisis (sélecteurs natifs)", async () => {
  pickExecutable.mockResolvedValue("C:\\Python\\python.exe");
  pickFolder.mockResolvedValue("D:\\MonCache");
  await renderScreen();

  const [, pythonBrowse, cacheBrowse] = screen.getAllByRole("button", {
    name: "Parcourir…",
  });
  fireEvent.click(pythonBrowse!);
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Chemin de Python" })).toHaveValue(
      "C:\\Python\\python.exe",
    ),
  );
  fireEvent.click(cacheBrowse!);
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Dossier de cache" })).toHaveValue(
      "D:\\MonCache",
    ),
  );

  fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));

  expect(await screen.findByText("Paramètres enregistrés")).toBeInTheDocument();
  expect(callsOf("settings_set")[0]?.args.settings).toEqual({
    schema_version: 1,
    player_path: null,
    python_path: "C:\\Python\\python.exe",
    cache_dir: "D:\\MonCache",
    default_catalog_url: "https://exemple.org/c.json",
    theme: "system",
    max_downloads: 2,
  });
});

it("remet un chemin en automatique avec le bouton dédié", async () => {
  view = makeView();
  view.settings.player_path = "C:\\x\\flam-player.exe";
  await renderScreen();
  const player = screen.getByRole("textbox", { name: "Chemin de flam-player" });
  expect(player).toHaveValue("C:\\x\\flam-player.exe");

  fireEvent.click(screen.getAllByRole("button", { name: "Automatique" })[0]!);
  expect(player).toHaveValue("");
  fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));

  await waitFor(() => expect(callsOf("settings_set")).toHaveLength(1));
  expect(callsOf("settings_set")[0]?.args.settings.player_path).toBeNull();
});

it("affiche l'erreur de validation renvoyée par le backend", async () => {
  await renderScreen();
  fireEvent.change(screen.getByRole("textbox", { name: "Chemin de flam-player" }), {
    target: { value: "C:\\absent\\flam-player.exe" },
  });
  failures.settings_set = { kind: "validation", message: "le fichier n'existe pas" };

  fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Valeur refusée");
  expect(alert).toHaveTextContent("le fichier n'existe pas");
});

it("change le thème, l'applique et l'enregistre", async () => {
  await renderScreen();

  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));

  expect(document.documentElement.dataset.theme).toBe("dark");
  await waitFor(() => expect(callsOf("settings_set")).toHaveLength(1));
  expect(callsOf("settings_set")[0]?.args.settings.theme).toBe("dark");
  expect(screen.getByRole("radio", { name: "Sombre" })).toBeChecked();

  fireEvent.click(screen.getByRole("radio", { name: "Système" }));
  await waitFor(() => expect(document.documentElement.dataset.theme).toBeUndefined());
});

it("rétablit le thème précédent si l'enregistrement échoue", async () => {
  await renderScreen();
  failures.settings_set = { kind: "io", message: "disque plein" };

  fireEvent.click(screen.getByRole("radio", { name: "Clair" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Le thème n'a pas pu être enregistré.",
  );
  expect(screen.getByRole("radio", { name: "Système" })).toBeChecked();
  expect(document.documentElement.dataset.theme).toBeUndefined();
});

it("deux échecs rapprochés restaurent le dernier thème enregistré", async () => {
  await renderScreen();
  pendingSets = [];
  fireEvent.click(screen.getByRole("radio", { name: "Clair" }));
  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));
  await waitFor(() => expect(pendingSets).toHaveLength(2));
  expect(document.documentElement.dataset.theme).toBe("dark");

  // Le second échoue d'abord, puis le premier : le thème revient à celui enregistré.
  pendingSets?.[1]?.reject();
  await screen.findByRole("alert");
  pendingSets?.[0]?.reject();
  await waitFor(() =>
    expect(screen.getByRole("radio", { name: "Système" })).toBeChecked(),
  );
  expect(document.documentElement.dataset.theme).toBeUndefined();
});

it("une réponse ancienne réussie après l'échec de la dernière resynchronise l'affichage", async () => {
  await renderScreen();
  pendingSets = [];
  fireEvent.click(screen.getByRole("radio", { name: "Clair" }));
  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));
  await waitFor(() => expect(pendingSets).toHaveLength(2));

  pendingSets?.[1]?.reject();
  await screen.findByRole("alert");
  expect(screen.getByRole("radio", { name: "Système" })).toBeChecked();

  pendingSets?.[0]?.resolve();
  await waitFor(() => expect(screen.getByRole("radio", { name: "Clair" })).toBeChecked());
  expect(document.documentElement.dataset.theme).toBe("light");
  expect(view.settings.theme).toBe("light");
});

it("un échec ancien n'annule pas un choix plus récent enregistré", async () => {
  await renderScreen();
  pendingSets = [];
  fireEvent.click(screen.getByRole("radio", { name: "Clair" }));
  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));
  await waitFor(() => expect(pendingSets).toHaveLength(2));

  pendingSets?.[1]?.resolve();
  await waitFor(() => expect(view.settings.theme).toBe("dark"));
  pendingSets?.[0]?.reject();
  await waitFor(() =>
    expect(screen.getByRole("radio", { name: "Sombre" })).toBeChecked(),
  );
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("vide le cache seulement après confirmation (focus sur Annuler)", async () => {
  await renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Vider le cache" }));

  const dialog = await screen.findByRole("alertdialog", { name: "Vider le cache ?" });
  expect(within(dialog).getByRole("button", { name: "Annuler" })).toHaveFocus();
  expect(dialog).toHaveAccessibleDescription(
    /archives, catalogs, library, thumbs.*C:\\cache.*2 Ko/,
  );

  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  expect(callsOf("cache_clear")).toHaveLength(0);

  fireEvent.click(screen.getByRole("button", { name: "Vider le cache" }));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Vider le cache",
    }),
  );

  expect(await screen.findByText("Cache vidé : 2 Ko libérés")).toBeInTheDocument();
  expect(await screen.findByText("Le cache est vide (C:\\cache).")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Vider le cache" })).toBeDisabled();
});

it("affiche l'erreur si le vidage du cache échoue", async () => {
  await renderScreen();
  failures.cache_clear = { kind: "io", message: "fichier verrouillé" };
  fireEvent.click(screen.getByRole("button", { name: "Vider le cache" }));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Vider le cache",
    }),
  );

  expect(await screen.findByRole("alert")).toHaveTextContent("fichier verrouillé");
});

it("affiche l'erreur de chargement avec un bouton Réessayer", async () => {
  failures.settings_get = { kind: "config_version", message: "fichier trop récent" };
  render(
    <ToastProvider>
      <RecoveryProvider>
        <SettingsScreen />
      </RecoveryProvider>
    </ToastProvider>,
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Configuration trop récente",
  );

  delete failures.settings_get;
  fireEvent.click(screen.getByRole("button", { name: "Réessayer" }));
  expect(
    await screen.findByRole("textbox", { name: "Chemin de flam-player" }),
  ).toBeInTheDocument();
});

it("n'a aucune violation axe, y compris avec la confirmation ouverte", async () => {
  await renderScreen();
  const rules = ["region", "page-has-heading-one"];
  await expectNoAxeViolations(document.body, rules);

  fireEvent.click(screen.getByRole("button", { name: "Vider le cache" }));
  await screen.findByRole("alertdialog");
  await expectNoAxeViolations(document.body, rules);
});
