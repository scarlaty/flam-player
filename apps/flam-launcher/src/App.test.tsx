import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { App, DEMO_HASH } from "./App";
import { routeFromHash } from "./router";
import { expectNoAxeViolations } from "./test/axe";

const settingsView = {
  settings: {
    schema_version: 1,
    player_path: null,
    python_path: null,
    cache_dir: null,
    default_catalog_url: "https://exemple.org/c.json",
    theme: "dark",
  },
  resolved: { player_path: null, python_path: null, cache_dir: "C:\\cache" },
  issues: [],
  recovery: null,
};

function mockBackend() {
  mockIPC((cmd) => {
    switch (cmd) {
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
  });
}

function goTo(hash: string) {
  act(() => {
    window.location.hash = hash;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}

afterEach(() => {
  clearMocks();
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

it("l'accueil n'a aucune violation axe", async () => {
  mockBackend();
  render(<App />);
  await screen.findByText("flam-launcher — version 0.1.0");
  await expectNoAxeViolations();
});
