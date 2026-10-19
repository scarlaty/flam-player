import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, onTestFinished, vi } from "vitest";
import { ExperimentalBanner, RISKS_DOC_URL } from "./ExperimentalBanner";

afterEach(() => clearMocks());

function stubClipboard(writeText: () => Promise<void>) {
  const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
  onTestFinished(() => {
    if (original) Object.defineProperty(navigator, "clipboard", original);
    else Reflect.deleteProperty(navigator, "clipboard");
  });
}

it("ouvre la page des risques dans le navigateur", async () => {
  const calls: Array<{ cmd: string; args: unknown }> = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return null;
  });
  render(<ExperimentalBanner firmware={null} />);
  fireEvent.click(screen.getByRole("button", { name: "Ouvrir la page des risques" }));
  await vi.waitFor(() =>
    expect(calls).toEqual([{ cmd: "open_external", args: { url: RISKS_DOC_URL } }]),
  );
  expect(screen.queryByText(/Ouverture impossible/)).not.toBeInTheDocument();
});

it("si l'ouverture échoue, annonce l'échec et laisse « Copier le lien » en repli", async () => {
  mockIPC(() => {
    throw { kind: "io", message: "pas de navigateur" };
  });
  const writeText = vi.fn().mockResolvedValue(undefined);
  stubClipboard(writeText);
  render(<ExperimentalBanner firmware={null} />);
  fireEvent.click(screen.getByRole("button", { name: "Ouvrir la page des risques" }));
  expect(await screen.findByText(/Ouverture impossible/)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Copier le lien des risques" }));
  expect(writeText).toHaveBeenCalledWith(RISKS_DOC_URL);
  expect(await screen.findByText(/Lien copié/)).toBeInTheDocument();
  expect(screen.queryByText(/Ouverture impossible/)).not.toBeInTheDocument();
});

it("l'adresse des risques est une page du dépôt du projet", () => {
  expect(RISKS_DOC_URL.startsWith("https://github.com/scarlaty/flam-player/")).toBe(true);
});
