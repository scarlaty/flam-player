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
    if (cmd === "authoring_editor_open")
      return { already_open: false, label: "editor-1" };
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
  mockIPC((cmd) =>
    cmd === "authoring_editor_open" ? { already_open: true, label: "editor-1" } : null,
  );
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
    openErrorText({
      kind: "root_refused",
      code: "folder_missing",
      message: "détail technique",
    }),
  ).toBe("Dossier introuvable : il a peut-être été déplacé ou renommé.");
  // Le message n'est jamais lu : sans code, un refus reste un refus.
  expect(
    openErrorText({ kind: "root_refused", message: "dossier introuvable : « C:\\x »" }),
  ).toContain("Ce dossier ne peut pas être utilisé");
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

it("« Modifier » : dossier introuvable, « Choisir le dossier » le retient puis ouvre l'éditeur", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "authoring_editor_open") {
      const path = (args as { path: string }).path;
      if (path === "C:\\Histoires\\Loup")
        throw {
          kind: "root_refused",
          code: "folder_missing",
          message: "détail technique",
        };
      return { already_open: false, label: "editor-1" };
    }
    if (cmd === "plugin:dialog|open") return "D:\\Loup";
    if (cmd === "authoring_relocate") return "D:\\Loup (résolu)";
    return null;
  });
  render(
    <ToastProvider>
      <OpenEditorButton folder="C:\Histoires\Loup" uuid="u-1" size="sm" />
    </ToastProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Modifier" }));
  fireEvent.click(await screen.findByRole("button", { name: "Choisir le dossier" }));
  await waitFor(() =>
    expect(calls.filter((c) => c.cmd === "authoring_editor_open").at(-1)?.args).toEqual({
      path: "D:\\Loup (résolu)",
    }),
  );
  expect(calls.find((c) => c.cmd === "authoring_relocate")?.args).toEqual({
    uuid: "u-1",
    path: "D:\\Loup",
  });
});

it("« Modifier » : dossier choisi qui n'est pas celui de l'histoire, message et rien d'ouvert", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "authoring_editor_open")
      throw { kind: "root_refused", code: "folder_missing", message: "x" };
    if (cmd === "plugin:dialog|open") return "D:\\Autre";
    if (cmd === "authoring_relocate")
      throw { kind: "authoring", code: "uuid_mismatch", message: "x" };
    return null;
  });
  render(<OpenEditorButton folder="C:\Histoires\Loup" uuid="u-1" size="sm" />);
  fireEvent.click(screen.getByRole("button", { name: "Modifier" }));
  fireEvent.click(await screen.findByRole("button", { name: "Choisir le dossier" }));
  expect(
    await screen.findByText(/n'est pas celui de cette histoire/),
  ).toBeInTheDocument();
  expect(calls.filter((c) => c === "authoring_editor_open")).toHaveLength(1);
});
