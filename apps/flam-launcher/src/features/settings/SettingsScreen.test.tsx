/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SettingsView } from "../../api/settings";
import type { VoiceList } from "../../api/speech";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { RecoveryProvider } from "../recovery/RecoveryProvider";
import { getMinimizeDuringPlay, setMinimizeDuringPlay } from "./preferences";
import { SettingsScreen } from "./SettingsScreen";

const pickExecutable = vi.hoisted(() => vi.fn());
const pickFolder = vi.hoisted(() => vi.fn());
vi.mock("../../api/dialog", () => ({ pickExecutable, pickFolder }));

function makeView(overrides: Partial<SettingsView> = {}): SettingsView {
  return {
    settings: {
      schema_version: 1,
      player_path: null,
      cache_dir: null,
      default_catalog_url: "https://exemple.org/c.json",
      theme: "system",
      max_downloads: 2,
      thumb_fit: "flou",
    },
    resolved: {
      player_path: "C:\\app\\flam-player.exe",
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

const HORTENSE = { id: "v-hortense", name: "Microsoft Hortense", language: "fr-FR" };
const PAUL = { id: "v-paul", name: "Microsoft Paul", language: "fr-CA" };
let voices: VoiceList;

beforeEach(() => {
  voices = { voices: [HORTENSE, PAUL], default_id: HORTENSE.id };
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
      case "tts_voices":
        return voices;
      case "tts_preview":
        return { played: true, duration_ms: 60_000 };
      case "tts_preview_stop":
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
    issues: [{ field: "player_path", kind: "not_found", message: "fichier disparu" }],
  });
  await renderScreen();

  expect(screen.getByText("Utilisé : C:\\app\\flam-player.exe")).toBeInTheDocument();
  expect(screen.queryByText(/Aucun fichier trouvé/)).toBeNull();
  expect(screen.getByText("fichier disparu")).toBeInTheDocument();
  expect(screen.queryByText(/Python/)).toBeNull();
  expect(await screen.findByText("flam-launcher — version 0.1.0")).toBeInTheDocument();
  expect(screen.getByText("Licence GPL v3.")).toBeInTheDocument();
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
  pickExecutable.mockResolvedValue("C:\\app\\flam-player.exe");
  pickFolder.mockResolvedValue("D:\\MonCache");
  await renderScreen();

  const [playerBrowse, cacheBrowse] = screen.getAllByRole("button", {
    name: "Parcourir…",
  });
  fireEvent.click(playerBrowse!);
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Chemin de flam-player" })).toHaveValue(
      "C:\\app\\flam-player.exe",
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
    player_path: "C:\\app\\flam-player.exe",
    cache_dir: "D:\\MonCache",
    default_catalog_url: "https://exemple.org/c.json",
    theme: "system",
    max_downloads: 2,
    thumb_fit: "flou",
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

it("propose le remplissage des vignettes (flou par défaut) et l'enregistre", async () => {
  await renderScreen();
  expect(screen.getByRole("radio", { name: /Flou/ })).toBeChecked();
  expect(
    screen.getByRole("group", { name: "Vignettes : remplissage" }),
  ).toBeInTheDocument();

  fireEvent.click(screen.getByRole("radio", { name: /Rogner/ }));

  await waitFor(() => expect(callsOf("settings_set")).toHaveLength(1));
  expect(callsOf("settings_set")[0]?.args.settings.thumb_fit).toBe("rogner");
  await waitFor(() => expect(view.settings.thumb_fit).toBe("rogner"));
  expect(screen.getByRole("radio", { name: /Rogner/ })).toBeChecked();

  fireEvent.click(screen.getByRole("radio", { name: /Bandes/ }));
  await waitFor(() => expect(view.settings.thumb_fit).toBe("bandes"));
});

it("bloque les vignettes pendant l'enregistrement du thème, et inversement", async () => {
  await renderScreen();
  const themeGroup = screen.getByRole("group", { name: "Thème" });
  const thumbGroup = screen.getByRole("group", { name: "Vignettes : remplissage" });
  pendingSets = [];

  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));
  await waitFor(() => expect(pendingSets).toHaveLength(1));
  expect(thumbGroup).toBeDisabled();
  expect(themeGroup).toBeEnabled();
  pendingSets?.[0]?.resolve();
  await waitFor(() => expect(thumbGroup).toBeEnabled());

  fireEvent.click(screen.getByRole("radio", { name: /Rogner/ }));
  await waitFor(() => expect(pendingSets).toHaveLength(2));
  expect(themeGroup).toBeDisabled();
  pendingSets?.[1]?.resolve();
  await waitFor(() => expect(themeGroup).toBeEnabled());
  // Chaque enregistrement conserve le réglage de l'autre.
  expect(view.settings.theme).toBe("dark");
  expect(view.settings.thumb_fit).toBe("rogner");
});

it("rétablit le remplissage précédent si l'enregistrement échoue", async () => {
  await renderScreen();
  failures.settings_set = { kind: "io", message: "disque plein" };

  fireEvent.click(screen.getByRole("radio", { name: /Bandes/ }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Le remplissage des vignettes n'a pas pu être enregistré.",
  );
  expect(screen.getByRole("radio", { name: /Flou/ })).toBeChecked();
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

it("réduire la fenêtre pendant la lecture : décoché par défaut, mémorisé localement", async () => {
  window.localStorage.clear();
  await renderScreen();
  const box = screen.getByRole("checkbox", {
    name: "Réduire la fenêtre pendant la lecture",
  });
  expect(box).not.toBeChecked();
  fireEvent.click(box);
  expect(box).toBeChecked();
  expect(getMinimizeDuringPlay()).toBe(true);
  fireEvent.click(box);
  expect(getMinimizeDuringPlay()).toBe(false);
  // Aucune écriture dans settings.json : c'est une préférence propre au poste.
  expect(callsOf("settings_set")).toHaveLength(0);
});

const voiceSelect = () => screen.findByRole("combobox", { name: "Voix" });

it("voix des titres lus : voix par défaut proposée, choix enregistré dans tts_voice", async () => {
  await renderScreen();
  const select = await voiceSelect();
  expect(select).toHaveValue(HORTENSE.id);
  expect(
    within(select)
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual(["Microsoft Hortense (fr-FR)", "Microsoft Paul (fr-CA)"]);
  // Pas de réglage de débit (v1).
  expect(screen.queryByText(/débit/i, { selector: "label" })).toBeNull();

  fireEvent.change(select, { target: { value: PAUL.id } });
  await waitFor(() => expect(callsOf("settings_set")).toHaveLength(1));
  expect(callsOf("settings_set")[0]!.args.settings).toEqual({
    ...makeView().settings,
    tts_voice: PAUL.id,
  });
  await waitFor(() => expect(select).toHaveValue(PAUL.id));
  await expectNoAxeViolations(document.body, ["region", "page-has-heading-one"]);
});

it("voix mémorisée relue ; voix disparue : la voix par défaut est annoncée", async () => {
  view = makeView();
  view.settings.tts_voice = PAUL.id;
  const { unmount } = await renderScreen();
  expect(await voiceSelect()).toHaveValue(PAUL.id);
  unmount();

  view.settings.tts_voice = "v-disparue";
  await renderScreen();
  const select = await voiceSelect();
  expect(select).toHaveValue("");
  expect(
    screen.getByText(
      "La voix choisie n'est plus installée : la voix par défaut sera utilisée.",
    ),
  ).toBeInTheDocument();
  // Choisir la voix de repli l'enregistre et efface le message.
  fireEvent.change(select, { target: { value: HORTENSE.id } });
  await waitFor(() =>
    expect(callsOf("settings_set").at(-1)?.args.settings.tts_voice).toBe(HORTENSE.id),
  );
  await waitFor(() =>
    expect(screen.queryByText(/La voix choisie n'est plus installée/)).toBeNull(),
  );
  expect(select).toHaveValue(HORTENSE.id);
});

it("voix : un échec d'enregistrement rétablit le choix précédent", async () => {
  await renderScreen();
  const select = await voiceSelect();
  failures.settings_set = { kind: "io", message: "disque plein" };
  fireEvent.change(select, { target: { value: PAUL.id } });
  expect(
    await screen.findByText("La voix n'a pas pu être enregistrée."),
  ).toBeInTheDocument();
  expect(select).toHaveValue(HORTENSE.id);
});

it("voix, thème et vignettes ne s'enregistrent pas en même temps (objet complet)", async () => {
  pendingSets = [];
  await renderScreen();
  const select = await voiceSelect();
  fireEvent.change(select, { target: { value: PAUL.id } });
  await waitFor(() => expect(pendingSets).toHaveLength(1));
  expect(screen.getByRole("radio", { name: "Sombre" })).toBeDisabled();
  expect(screen.getByRole("radio", { name: /^Rogner/ })).toBeDisabled();
  pendingSets[0]!.resolve();
  await waitFor(() =>
    expect(screen.getByRole("radio", { name: "Sombre" })).toBeEnabled(),
  );
  // Le thème enregistré ensuite garde la voix choisie.
  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));
  await waitFor(() => expect(pendingSets).toHaveLength(2));
  expect(select).toBeDisabled();
  expect(pendingSets.length).toBe(2);
  pendingSets[1]!.resolve();
  await waitFor(() => expect(select).toBeEnabled());
  expect(view.settings).toMatchObject({ theme: "dark", tts_voice: PAUL.id });
});

it("essayer la voix choisie, puis arrêter", async () => {
  view = makeView();
  view.settings.tts_voice = PAUL.id;
  await renderScreen();
  await voiceSelect();
  fireEvent.click(screen.getByRole("button", { name: "Essayer la voix" }));
  const stop = await screen.findByRole("button", { name: "Arrêter" });
  expect(callsOf("tts_preview")[0]!.args).toEqual({
    text: "Bonjour ! Je lirai les titres de vos histoires.",
    voice: PAUL.id,
  });
  fireEvent.click(stop);
  expect(await screen.findByRole("button", { name: "Essayer la voix" })).toBeEnabled();
  await waitFor(() => expect(callsOf("tts_preview_stop")).toHaveLength(1));
});

it("sans voix française : explication, ni liste ni essai", async () => {
  voices = { voices: [], default_id: null };
  await renderScreen();
  const message = await screen.findByText(/Aucune voix française n'est installée/);
  expect(message.textContent).toContain("Heure et langue > Voix");
  expect(screen.queryByRole("combobox", { name: "Voix" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Essayer la voix" })).toBeNull();
});

it("liste des voix en échec : message, le reste de l'écran fonctionne", async () => {
  failures.tts_voices = { kind: "tts_failed", message: "panne" };
  await renderScreen();
  expect(
    await screen.findByText(/Les voix de Windows n'ont pas pu être listées/),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Enregistrer" })).toBeEnabled();
});

it("relit la préférence enregistrée", async () => {
  setMinimizeDuringPlay(true);
  await renderScreen();
  expect(
    screen.getByRole("checkbox", { name: "Réduire la fenêtre pendant la lecture" }),
  ).toBeChecked();
  window.localStorage.clear();
});
