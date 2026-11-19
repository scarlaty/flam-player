import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { ToastProvider } from "../../../components";
import { openErrorText } from "./errors";
import { OpenEditorButton } from "./OpenEditorButton";

afterEach(() => clearMocks());

it("choisir un dossier ouvre l'éditeur sur ce dossier", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "plugin:dialog|open") return "C:\\Histoires\\Loup";
    if (cmd === "authoring_editor_open") return { already_open: false };
    return null;
  });
  render(<OpenEditorButton />);
  fireEvent.click(
    screen.getByRole("button", { name: "Créer une histoire depuis un dossier" }),
  );
  await waitFor(() =>
    expect(calls.find((c) => c.cmd === "authoring_editor_open")?.args).toEqual({
      path: "C:\\Histoires\\Loup",
    }),
  );
});

it("« Modifier » : dossier déjà ouvert, sa fenêtre revient avec un message", async () => {
  mockIPC((cmd) => (cmd === "authoring_editor_open" ? { already_open: true } : null));
  render(
    <ToastProvider>
      <OpenEditorButton folder="C:\\Histoires\\Loup" size="sm" />
    </ToastProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Modifier" }));
  expect(await screen.findByText(/déjà ouvert dans un éditeur/)).toBeInTheDocument();
});

it("dossier refusé ou disparu : message sans jargon", () => {
  expect(
    openErrorText({ kind: "root_refused", message: "dossier introuvable : « C:\\x »" }),
  ).toBe("Dossier introuvable : il a peut-être été déplacé ou renommé.");
  expect(
    openErrorText({
      kind: "root_refused",
      message: "« C:\\ » ne peut pas être le dossier",
    }),
  ).toContain("Ce dossier ne peut pas être utilisé");
  expect(openErrorText({ kind: "window", message: "x" })).toContain(
    "n'a pas pu s'ouvrir",
  );
});
