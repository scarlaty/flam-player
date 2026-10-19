import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { DiagnosticPanel, REPORT_ISSUE_URL } from "./DiagnosticPanel";

const DIAGNOSTIC = "## Diagnostic flam-launcher\n\n- Launcher : 0.1.0\n";

let failure: { kind: string; message: string } | null;
let calls: number;
let opened: string[];
let openFailure: { kind: string; message: string } | null;

beforeEach(() => {
  failure = null;
  calls = 0;
  opened = [];
  openFailure = null;
  mockIPC((cmd, args) => {
    if (cmd === "open_external") {
      if (openFailure) throw openFailure;
      opened.push((args as { url: string }).url);
      return null;
    }
    if (cmd !== "device_diagnostic") throw { kind: "unknown", message: cmd };
    calls += 1;
    if (failure) throw failure;
    return DIAGNOSTIC;
  });
});

afterEach(() => {
  clearMocks();
});

function mockClipboard(writeText: (text: string) => Promise<void>) {
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

it("copie le diagnostic dans le presse-papiers et l'annonce", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  mockClipboard(writeText);
  const { container } = render(<DiagnosticPanel />);
  expect(screen.getByRole("heading", { name: "Retour de test" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Copier le diagnostic" }));
  expect(await screen.findByText("Diagnostic copié")).toHaveAttribute(
    "aria-live",
    "polite",
  );
  expect(writeText).toHaveBeenCalledWith(DIAGNOSTIC);
  expect(calls).toBe(1);
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  await expectComponentAccessible(container);
});

it("montre le texte sélectionné si la copie échoue", async () => {
  mockClipboard(() => Promise.reject(new Error("refusé")));
  const { container } = render(<DiagnosticPanel />);
  fireEvent.click(screen.getByRole("button", { name: "Copier le diagnostic" }));
  const area = await screen.findByRole("textbox", { name: "Diagnostic à copier" });
  expect(area).toHaveValue(DIAGNOSTIC);
  expect(area).toHaveAttribute("readonly");
  expect(area).toHaveFocus();
  expect(screen.getByText(/Copie impossible/)).toHaveAttribute("aria-live", "polite");
  await expectComponentAccessible(container);
});

it("montre le texte si le presse-papiers est indisponible", async () => {
  const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
  onTestFinished(() => {
    if (original) Object.defineProperty(navigator, "clipboard", original);
    else Reflect.deleteProperty(navigator, "clipboard");
  });
  render(<DiagnosticPanel />);
  fireEvent.click(screen.getByRole("button", { name: "Copier le diagnostic" }));
  expect(await screen.findByRole("textbox", { name: "Diagnostic à copier" })).toHaveValue(
    DIAGNOSTIC,
  );
});

it("signale l'échec du diagnostic", async () => {
  mockClipboard(vi.fn().mockResolvedValue(undefined));
  failure = { kind: "io", message: "volume retiré" };
  render(<DiagnosticPanel />);
  fireEvent.click(screen.getByRole("button", { name: "Copier le diagnostic" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Diagnostic impossible : volume retiré",
  );
  // Nouvel essai possible.
  failure = null;
  fireEvent.click(screen.getByRole("button", { name: "Copier le diagnostic" }));
  expect(await screen.findByText("Diagnostic copié")).toBeInTheDocument();
  expect(calls).toBe(2);
});

it("ouvre le modèle d'issue « Retour testeur Flam »", async () => {
  const { container } = render(<DiagnosticPanel />);
  expect(REPORT_ISSUE_URL).toBe(
    "https://github.com/scarlaty/flam-player/issues/new?template=retour-testeur-flam.yml",
  );
  fireEvent.click(screen.getByRole("button", { name: "Signaler un problème" }));
  await vi.waitFor(() => expect(opened).toEqual([REPORT_ISSUE_URL]));
  expect(screen.getByText(REPORT_ISSUE_URL)).toBeInTheDocument();
  await expectComponentAccessible(container);
});

it("propose de copier le lien si l'ouverture échoue", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  mockClipboard(writeText);
  openFailure = { kind: "validation", message: "adresse refusée" };
  render(<DiagnosticPanel />);
  fireEvent.click(screen.getByRole("button", { name: "Signaler un problème" }));
  expect(await screen.findByText(/Ouverture impossible/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Copier le lien" }));
  expect(await screen.findByText(/Lien copié/)).toBeInTheDocument();
  expect(writeText).toHaveBeenCalledWith(REPORT_ISSUE_URL);
});
