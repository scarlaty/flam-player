import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { App, DEMO_HASH } from "./App";
import { routeFromHash } from "./router";
import { expectNoAxeViolations } from "./test/axe";
import { slowIt, tolerateLoadInAsyncUtils } from "./test/timeouts";

tolerateLoadInAsyncUtils();

const settingsView = {
  settings: {
    schema_version: 1,
    player_path: null,
    python_path: null,
    cache_dir: null,
    default_catalog_url: "https://exemple.org/c.json",
    theme: "dark",
    max_downloads: 2,
  },
  resolved: { player_path: null, python_path: null, cache_dir: "C:\\cache" },
  issues: [],
  recovery: null,
};

const embedded = {
  origin: "library",
  uuid: "u1",
  title: "Les trois ours",
  pid: 42,
  started_at: 1,
  embedded: true,
};

function mockBackend(playing: object | null = null, calls: string[] = []) {
  mockIPC(
    (cmd) => {
      calls.push(cmd);
      switch (cmd) {
        case "player_status":
          return playing;
        case "player_embed_layout":
          return null;
        case "app_info":
          return { name: "flam-launcher", version: "0.1.0" };
        case "settings_get":
          return settingsView;
        case "sources_list":
          return {
            sources: [{ type: "library", id: "library", name: "Mes histoires" }],
            recovery: null,
          };
        case "cache_usage":
          return { path: "C:\\cache", bytes: 0, subdirs: [] };
        default:
          throw { kind: "unknown", message: cmd };
      }
    },
    { shouldMockEvents: true },
  );
}

function goTo(hash: string) {
  act(() => {
    window.location.hash = hash;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}

afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
  window.location.hash = "";
  document.documentElement.removeAttribute("data-theme");
});

it("route les hash vers les écrans", () => {
  expect(routeFromHash("")).toBe("home");
  expect(routeFromHash("#/sources")).toBe("sources");
  expect(routeFromHash("#/settings")).toBe("settings");
  expect(routeFromHash("#/inconnu")).toBe("home");
});

it("affiche l'accueil, navigue vers les écrans puis vers la démonstration", async () => {
  mockBackend();
  render(<App />);

  expect(
    screen.getByRole("heading", { level: 1, name: "Bienvenue" }),
  ).toBeInTheDocument();
  expect(await screen.findByText("flam-launcher — version 0.1.0")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Démonstration/ })).toHaveAttribute(
    "href",
    DEMO_HASH,
  );
  expect(screen.getByRole("link", { name: "Accueil" })).toHaveAttribute(
    "aria-current",
    "page",
  );

  goTo("#/sources");
  expect(screen.getByRole("heading", { level: 1, name: "Sources" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Sources" })).toHaveAttribute(
    "aria-current",
    "page",
  );

  goTo("#/settings");
  expect(
    screen.getByRole("heading", { level: 1, name: "Paramètres" }),
  ).toBeInTheDocument();

  goTo(DEMO_HASH);
  expect(
    await screen.findByRole("heading", {
      level: 1,
      name: "Démonstration des composants",
    }),
  ).toBeInTheDocument();
});

it("applique le thème enregistré au démarrage", async () => {
  mockBackend();
  render(<App />);

  await screen.findByText("flam-launcher — version 0.1.0");
  await expect.poll(() => document.documentElement.dataset.theme).toBe("dark");
});

slowIt("l'accueil n'a aucune violation axe", async () => {
  mockBackend();
  render(<App />);
  await screen.findByText("flam-launcher — version 0.1.0");
  await expectNoAxeViolations();
});

it("un player incrusté remplace l'application par l'écran Lecture jusqu'à sa fin", async () => {
  class FakeResizeObserver {
    observe() {}
    disconnect() {}
  }
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  const calls: string[] = [];
  mockBackend(embedded, calls);
  render(<App />);

  expect(
    await screen.findByRole("heading", { level: 1, name: "Les trois ours" }),
  ).toBeInTheDocument();
  // Pas de navigation pendant la lecture.
  expect(screen.queryByRole("navigation")).toBeNull();
  await waitFor(() => expect(calls).toContain("player_embed_layout"));

  await act(() =>
    emit("player://exited", {
      session: embedded,
      code: 0,
      success: true,
      duration_ms: 1000,
      stderr: "",
      saves_error: null,
    }),
  );
  expect(
    await screen.findByRole("heading", { level: 1, name: "Bienvenue" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("navigation", { name: "Navigation" })).toBeInTheDocument();
});
