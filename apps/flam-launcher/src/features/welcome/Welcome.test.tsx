import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { Welcome } from "./Welcome";

afterEach(() => clearMocks());

it("affiche le nom et la version", async () => {
  mockIPC(() => ({ name: "flam-launcher", version: "0.1.0" }));

  render(<Welcome />);

  expect(await screen.findByText("flam-launcher — version 0.1.0")).toBeInTheDocument();
});

it("signale une erreur si la commande échoue", async () => {
  mockIPC(() => {
    throw new Error("échec");
  });

  render(<Welcome />);

  expect(await screen.findByRole("alert")).toBeInTheDocument();
});
