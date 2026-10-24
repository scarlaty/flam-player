/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, it } from "vitest";
import type { VoiceList } from "../../../api/speech";
import { expectComponentAccessible } from "../../../test/axe";
import { TitlesAudioField } from "./TitlesAudioField";

const HORTENSE = { id: "v-hortense", name: "Microsoft Hortense", language: "fr-FR" };
const PAUL = { id: "v-paul", name: "Microsoft Paul", language: "fr-FR" };

let voices: VoiceList | Error | null;
let preferred: string | null;
let preview: { played: boolean; duration_ms: number } | { kind: string; message: string };
let calls: { cmd: string; args: Record<string, any> }[];
// Réponse de `settings_get` : différée (porte) ou en échec.
let settingsGate: Promise<void> | null;
let settingsFails: boolean;

beforeEach(() => {
  voices = { voices: [HORTENSE, PAUL], default_id: HORTENSE.id };
  preferred = null;
  preview = { played: true, duration_ms: 60_000 };
  calls = [];
  settingsGate = null;
  settingsFails = false;
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, any> });
    switch (cmd) {
      case "tts_voices":
        if (voices === null) return new Promise(() => {});
        if (voices instanceof Error) throw { kind: "io", message: voices.message };
        return voices;
      case "settings_get":
        if (settingsFails) throw { kind: "io", message: "illisible" };
        if (settingsGate) {
          return settingsGate.then(() => ({ settings: { tts_voice: preferred } }));
        }
        return { settings: { tts_voice: preferred } };
      case "tts_preview":
        if ("kind" in preview) throw preview;
        return preview;
      case "tts_preview_stop":
        return null;
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => clearMocks());

function Harness({
  initial = false,
  voice = null,
  previewText = "Mon pack",
}: {
  initial?: boolean;
  voice?: string | null;
  previewText?: string;
}) {
  const [checked, setChecked] = useState(initial);
  return (
    <TitlesAudioField
      checked={checked}
      onChange={setChecked}
      voice={voice}
      previewText={previewText}
    />
  );
}

const checkbox = () =>
  screen.getByRole("checkbox", { name: "Lire les titres à voix haute" });
const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);

it("pendant la recherche des voix, la case est désactivée et l'attente annoncée", async () => {
  voices = null;
  render(<Harness />);
  expect(await screen.findByText("Recherche des voix françaises…")).toBeInTheDocument();
  expect(checkbox()).toBeDisabled();
  expect(screen.queryByRole("button", { name: /Écouter/ })).toBeNull();
});

it("sans voix française : case désactivée, explication et où ajouter une voix", async () => {
  voices = { voices: [], default_id: null };
  const { container } = render(<Harness />);
  const message = await screen.findByText(/Aucune voix française n'est installée/);
  expect(message.textContent).toContain("Heure et langue > Voix");
  expect(checkbox()).toBeDisabled();
  // L'explication est reliée à la case pour les lecteurs d'écran.
  expect(checkbox().getAttribute("aria-describedby")).toContain(message.id);
  expect(screen.queryByRole("button", { name: /Écouter/ })).toBeNull();
  await expectComponentAccessible(container);
});

it("sans voix française, une case déjà cochée (brouillon) reste décochable", async () => {
  voices = { voices: [], default_id: null };
  render(<Harness initial />);
  await screen.findByText(/Aucune voix française n'est installée/);
  expect(checkbox()).toBeEnabled();
  fireEvent.click(checkbox());
  expect(checkbox()).not.toBeChecked();
  expect(checkbox()).toBeDisabled();
});

it("liste des voix en échec : case désactivée avec un message", async () => {
  voices = new Error("service");
  render(<Harness />);
  expect(
    await screen.findByText(/Les voix de Windows n'ont pas pu être listées/),
  ).toBeInTheDocument();
  expect(checkbox()).toBeDisabled();
});

it("avec des voix : case activable, voix des Paramètres affichée", async () => {
  preferred = PAUL.id;
  const { container } = render(<Harness />);
  expect(await screen.findByText(/Voix : Microsoft Paul/)).toBeInTheDocument();
  expect(checkbox()).toBeEnabled();
  fireEvent.click(checkbox());
  expect(checkbox()).toBeChecked();
  await expectComponentAccessible(container);
});

it("voix des Paramètres disparue : la voix par défaut est annoncée", async () => {
  preferred = "v-disparue";
  render(<Harness />);
  expect(
    await screen.findByText(/n'est plus installée : Microsoft Hortense sera utilisée/),
  ).toBeInTheDocument();
});

it("la voix de la composition l'emporte sur celle des Paramètres", async () => {
  preferred = HORTENSE.id;
  render(<Harness voice={PAUL.id} />);
  expect(
    await screen.findByText("Voix : Microsoft Paul (celle de la composition)."),
  ).toBeInTheDocument();
});

it("écouter puis arrêter l'aperçu du titre du pack", async () => {
  preferred = PAUL.id;
  render(<Harness />);
  await screen.findByText(/Voix : Microsoft Paul/);
  fireEvent.click(screen.getByRole("button", { name: "Écouter le titre du pack" }));
  const stop = await screen.findByRole("button", {
    name: "Arrêter la lecture du titre du pack",
  });
  expect(callsOf("tts_preview")[0]?.args).toEqual({ text: "Mon pack", voice: PAUL.id });
  fireEvent.click(stop);
  expect(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  ).toBeInTheDocument();
  await waitFor(() => expect(callsOf("tts_preview_stop")).toHaveLength(1));
});

it("l'aperçu revient à « Écouter » à la fin du son", async () => {
  preview = { played: true, duration_ms: 20 };
  render(<Harness />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  await screen.findByRole("button", { name: "Arrêter la lecture du titre du pack" });
  expect(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  ).toBeInTheDocument();
  expect(callsOf("tts_preview_stop")).toHaveLength(0);
});

it("aperçu abandonné par le backend (rien joué) : retour à « Écouter »", async () => {
  preview = { played: false, duration_ms: 0 };
  render(<Harness />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  await waitFor(() => expect(callsOf("tts_preview")).toHaveLength(1));
  expect(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  ).toBeInTheDocument();
});

it("erreur d'aperçu traduite par code", async () => {
  preview = { kind: "tts_failed", message: "La synthèse vocale a échoué (détail)." };
  render(<Harness />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "La voix n'a pas pu lire ce texte. Réessayez.",
  );
});

it("code d'erreur inconnu ou hérité : message du backend", async () => {
  preview = { kind: "constructor", message: "Ce titre n'a rien à lire à voix haute." };
  render(<Harness />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Ce titre n'a rien à lire à voix haute.",
  );
});

it("sans titre de pack, l'aperçu est désactivé avec une explication", async () => {
  render(<Harness previewText="   " />);
  const button = await screen.findByRole("button", { name: "Écouter le titre du pack" });
  // aria-disabled : le bouton reste focalisable, et un clic ne lance rien.
  expect(button).toHaveAttribute("aria-disabled", "true");
  expect(button).toBeEnabled();
  expect(button).toHaveAccessibleDescription(
    "Saisissez un titre de pack pour l'écouter.",
  );
  fireEvent.click(button);
  expect(callsOf("tts_preview")).toHaveLength(0);
});

it("titre vidé pendant l'aperçu : le bouton garde le focus", async () => {
  function Emptying() {
    const [title, setTitle] = useState("Mon pack");
    return (
      <>
        <button onClick={() => setTitle("")}>vider</button>
        <TitlesAudioField
          checked={false}
          onChange={() => {}}
          voice={null}
          previewText={title}
        />
      </>
    );
  }
  render(<Emptying />);
  const listen = await screen.findByRole("button", { name: "Écouter le titre du pack" });
  fireEvent.click(listen);
  const stop = await screen.findByRole("button", {
    name: "Arrêter la lecture du titre du pack",
  });
  stop.focus();
  expect(stop).toHaveFocus();
  fireEvent.click(screen.getByRole("button", { name: "vider" }));
  const back = await screen.findByRole("button", { name: "Écouter le titre du pack" });
  expect(back).toHaveAttribute("aria-disabled", "true");
  // Le focus n'est pas tombé sur le corps de la page.
  expect(back).toHaveFocus();
});

it("voix des Paramètres pas encore lue : l'aperçu attend au lieu de jouer la voix par défaut", async () => {
  preferred = PAUL.id;
  let release = () => {};
  settingsGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  render(<Harness />);
  expect(
    await screen.findByText("Lecture de la voix choisie dans les Paramètres…"),
  ).toBeInTheDocument();
  const button = screen.getByRole("button", { name: "Écouter le titre du pack" });
  expect(button).toHaveAttribute("aria-disabled", "true");
  // L'attente est expliquée au clavier et aux lecteurs d'écran.
  expect(button).toHaveAccessibleDescription(
    "Lecture de la voix choisie dans les Paramètres…",
  );
  fireEvent.click(button);
  expect(callsOf("tts_preview")).toHaveLength(0);

  release();
  await screen.findByText(/Voix : Microsoft Paul/);
  expect(button).not.toHaveAttribute("aria-disabled");
  fireEvent.click(button);
  await waitFor(() =>
    expect(callsOf("tts_preview")[0]?.args).toEqual({ text: "Mon pack", voice: PAUL.id }),
  );
});

it("voix de la composition : l'aperçu n'attend pas les Paramètres", async () => {
  settingsGate = new Promise<void>(() => {});
  render(<Harness voice={PAUL.id} />);
  await screen.findByText("Voix : Microsoft Paul (celle de la composition).");
  fireEvent.click(screen.getByRole("button", { name: "Écouter le titre du pack" }));
  await waitFor(() =>
    expect(callsOf("tts_preview")[0]?.args).toEqual({ text: "Mon pack", voice: PAUL.id }),
  );
});

it("lecture des Paramètres en échec : repli sur la voix par défaut, signalé", async () => {
  preferred = PAUL.id;
  settingsFails = true;
  render(<Harness />);
  expect(
    await screen.findByText(
      "La voix choisie dans les Paramètres n'a pas pu être lue : Microsoft Hortense sera utilisée.",
    ),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Écouter le titre du pack" }));
  await waitFor(() =>
    expect(callsOf("tts_preview")[0]?.args).toEqual({
      text: "Mon pack",
      voice: HORTENSE.id,
    }),
  );
});

it("l'aperçu en cours est coupé quand le titre change ou que la case disparaît", async () => {
  function Changing() {
    const [title, setTitle] = useState("Mon pack");
    const [shown, setShown] = useState(true);
    return (
      <>
        <button onClick={() => setTitle("Autre titre")}>changer</button>
        <button onClick={() => setShown(false)}>fermer</button>
        {shown && (
          <TitlesAudioField
            checked={false}
            onChange={() => {}}
            voice={null}
            previewText={title}
          />
        )}
      </>
    );
  }
  render(<Changing />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  );
  await screen.findByRole("button", { name: "Arrêter la lecture du titre du pack" });
  fireEvent.click(screen.getByRole("button", { name: "changer" }));
  expect(
    await screen.findByRole("button", { name: "Écouter le titre du pack" }),
  ).toBeInTheDocument();
  await waitFor(() => expect(callsOf("tts_preview_stop")).toHaveLength(1));

  fireEvent.click(screen.getByRole("button", { name: "Écouter le titre du pack" }));
  await screen.findByRole("button", { name: "Arrêter la lecture du titre du pack" });
  fireEvent.click(screen.getByRole("button", { name: "fermer" }));
  await waitFor(() => expect(callsOf("tts_preview_stop")).toHaveLength(2));
});
