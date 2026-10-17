/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { PlayerSession } from "../../api/player";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { GONE_GRACE_MS, PlayerScreen, STOP_TIMEOUT_MS } from "./PlayerScreen";

const session: PlayerSession = {
  origin: "library",
  uuid: "u1",
  title: "Les trois ours",
  pid: 42,
  started_at: 1,
  embedded: true,
};

let calls: { cmd: string; args: Record<string, any> }[];
let stopFails: boolean;
let stopResult: boolean;
let onGone: ReturnType<typeof vi.fn<() => void>>;
let resize: (() => void) | null;

beforeEach(() => {
  calls = [];
  stopFails = false;
  stopResult = true;
  onGone = vi.fn<() => void>();
  resize = null;
  class FakeResizeObserver {
    constructor(callback: () => void) {
      resize = callback;
    }
    observe() {}
    disconnect() {}
  }
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, any> });
    if (cmd === "player_stop") {
      if (stopFails) throw { kind: "io", message: "refusé" };
      return stopResult;
    }
    return null;
  });
});

afterEach(() => {
  vi.useRealTimers();
  clearMocks();
  vi.unstubAllGlobals();
});

const layouts = () => calls.filter((c) => c.cmd === "player_embed_layout");

function renderScreen() {
  return render(
    <ToastProvider>
      <PlayerScreen session={session} onGone={onGone} />
    </ToastProvider>,
  );
}

it("affiche le titre, la zone du lecteur et l'aide, sans violation axe", async () => {
  renderScreen();
  expect(
    screen.getByRole("heading", { level: 1, name: "Les trois ours" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("region", { name: "Lecteur : Les trois ours" }),
  ).toBeInTheDocument();
  expect(screen.getByText(/Cliquez dans le lecteur/)).toBeInTheDocument();
  // Focus au montage sur le titre (l'élément qui a lancé la lecture a disparu).
  expect(screen.getByRole("heading", { level: 1 })).toHaveFocus();
  await expectNoAxeViolations();
});

it("« Clavier au lecteur » donne le clavier au player incrusté", async () => {
  renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Clavier au lecteur" }));
  await waitFor(() =>
    expect(calls.some((c) => c.cmd === "player_embed_focus")).toBe(true),
  );
});

it("aucun player côté backend : l'écran se ferme après un court délai (pas de blocage)", async () => {
  stopResult = false;
  renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Arrêter" }));
  await waitFor(() => expect(calls.some((c) => c.cmd === "player_stop")).toBe(true));
  // Délai de grâce : player://exited peut encore arriver.
  expect(onGone).not.toHaveBeenCalled();
  await waitFor(() => expect(onGone).toHaveBeenCalledTimes(1), {
    timeout: GONE_GRACE_MS + 2000,
  });
}, 10_000);

it("sans fin de partie après le délai, Arrêter redevient disponible avec une alerte", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Arrêter" }));
  expect(screen.getByRole("button", { name: "Arrêt en cours…" })).toBeDisabled();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(STOP_TIMEOUT_MS);
  });
  expect(screen.getByRole("alert")).toHaveTextContent(/ne répond pas/);
  expect(screen.getByRole("button", { name: "Arrêter" })).toBeEnabled();
  expect(onGone).not.toHaveBeenCalled();
});

it("envoie la zone au montage, à chaque changement, et la cache au démontage", async () => {
  const view = renderScreen();
  const zone = screen.getByRole("region", { name: "Lecteur : Les trois ours" });
  let box = { left: 10, top: 20, width: 300, height: 200 };
  zone.getBoundingClientRect = () =>
    ({ ...box, right: 0, bottom: 0, x: 0, y: 0 }) as DOMRect;
  // La mesure du montage a eu lieu avant le remplacement : un changement en déclenche une autre.
  await waitFor(() => expect(layouts()).toHaveLength(1));
  box = { left: 10, top: 20, width: 640, height: 480 };
  resize?.();
  await waitFor(() =>
    expect(layouts().at(-1)?.args).toEqual({
      rect: { x: 10, y: 20, width: 640, height: 480 },
    }),
  );
  // Même rectangle : rien n'est renvoyé.
  const count = layouts().length;
  window.dispatchEvent(new Event("resize"));
  await new Promise((resolve) => requestAnimationFrame(resolve));
  expect(layouts()).toHaveLength(count);
  view.unmount();
  expect(layouts().at(-1)?.args).toEqual({ rect: null });
});

it("Arrêter appelle player_stop et se désactive", async () => {
  renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Arrêter" }));
  await waitFor(() => expect(calls.some((c) => c.cmd === "player_stop")).toBe(true));
  expect(screen.getByRole("button", { name: "Arrêt en cours…" })).toBeDisabled();
});

it("un arrêt refusé réactive le bouton et l'erreur s'affiche en alerte hors de la zone", async () => {
  stopFails = true;
  renderScreen();
  fireEvent.click(screen.getByRole("button", { name: "Arrêter" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    /flam-player n'a pas pu être arrêté\. refusé/,
  );
  expect(screen.getByRole("button", { name: "Arrêter" })).toBeEnabled();
});
