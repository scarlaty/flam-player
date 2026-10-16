import { afterEach, expect, it, vi } from "vitest";
import { applyRememberedTheme, applyTheme, readRememberedTheme } from "./theme";

afterEach(() => {
  document.documentElement.removeAttribute("data-theme");
  window.localStorage.clear();
  vi.restoreAllMocks();
});

it("force le thème clair ou sombre, ou rend la main au système", () => {
  const root = document.documentElement;

  applyTheme("dark");
  expect(root.dataset.theme).toBe("dark");
  applyTheme("light");
  expect(root.dataset.theme).toBe("light");
  applyTheme("system");
  expect(root).not.toHaveAttribute("data-theme");
});

it("mémorise le thème et le réapplique avant le chargement des paramètres", () => {
  applyTheme("dark");
  expect(readRememberedTheme()).toBe("dark");
  document.documentElement.removeAttribute("data-theme");
  applyRememberedTheme();
  expect(document.documentElement.dataset.theme).toBe("dark");
});

it("ignore une valeur mémorisée invalide ou un stockage indisponible", () => {
  window.localStorage.setItem("flam-launcher.theme", "rose");
  applyRememberedTheme();
  expect(document.documentElement).not.toHaveAttribute("data-theme");

  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("indisponible");
  });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("indisponible");
  });
  expect(readRememberedTheme()).toBeNull();
  expect(() => applyTheme("light")).not.toThrow();
  expect(document.documentElement.dataset.theme).toBe("light");
});
