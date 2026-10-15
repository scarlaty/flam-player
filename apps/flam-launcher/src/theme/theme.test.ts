import { afterEach, expect, it } from "vitest";
import { applyTheme } from "./theme";

afterEach(() => document.documentElement.removeAttribute("data-theme"));

it("force le thème clair ou sombre, ou rend la main au système", () => {
  const root = document.documentElement;

  applyTheme("dark");
  expect(root.dataset.theme).toBe("dark");
  applyTheme("light");
  expect(root.dataset.theme).toBe("light");
  applyTheme("system");
  expect(root).not.toHaveAttribute("data-theme");
});
