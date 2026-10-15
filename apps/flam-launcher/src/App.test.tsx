import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { App, DEMO_HASH } from "./App";

afterEach(() => {
  clearMocks();
  window.location.hash = "";
});

it("affiche l'accueil puis la démonstration sur #/demo (développement)", async () => {
  mockIPC(() => ({ name: "flam-launcher", version: "0.1.0" }));
  render(<App />);

  expect(screen.getByRole("heading", { name: "Bienvenue" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Démonstration/ })).toHaveAttribute(
    "href",
    DEMO_HASH,
  );

  act(() => {
    window.location.hash = DEMO_HASH;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
  expect(
    await screen.findByRole("heading", {
      level: 1,
      name: "Démonstration des composants",
    }),
  ).toBeInTheDocument();
});
