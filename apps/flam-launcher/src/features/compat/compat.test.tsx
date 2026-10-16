import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CompatEntry, CompatReport } from "../../api/compat";
import type { Source } from "../../api/sources";
import { ToastProvider } from "../../components";
import { PAGE_LEVEL_RULES, expectNoAxeViolations } from "../../test/axe";
import { makeEntry } from "../../test/entries";
import { LibraryProvider } from "../library/LibraryProvider";
import { LibraryScreen } from "../library/LibraryScreen";
import { CompatNotice } from "./CompatNotice";
import { CompatProvider } from "./CompatProvider";
import { useCompat } from "./CompatContext";
import { describeCompat } from "./compatInfo";
import {
  IncompatibleInstallDialog,
  needsInstallWarning,
} from "./IncompatibleInstallDialog";
import { slowIt, tolerateLoadInAsyncUtils } from "../../test/timeouts";

tolerateLoadInAsyncUtils();

const folder: Source = { type: "folder", id: "d", name: "Contes", path: "C:\\Contes" };

function fiche(over: Partial<CompatEntry> & { uuid: string }): CompatEntry {
  return {
    version: null,
    level: "compatible",
    reason: null,
    code: "OK",
    origin: "campaign",
    stale: false,
    ...over,
  };
}

let report: CompatReport | "fail";

beforeEach(() => {
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  report = { entries: [], issue: null };
  mockIPC((cmd) => {
    switch (cmd) {
      case "sources_list":
        return { sources: [folder], recovery: null };
      case "folder_scan":
        return {
          entries: ["ok", "avert", "echec", "futur", "rien"].map((uuid) =>
            makeEntry({ title: `Histoire ${uuid}`, uuid, source_id: "d" }),
          ),
          warnings: [],
        };
      case "folder_thumbnail":
        return "data:image/png;base64,AAAA";
      case "compat_statuses":
        if (report === "fail") throw { kind: "io", message: "x" };
        return report;
      default:
        throw { kind: "unknown", message: cmd };
    }
  });
});

afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
});

function Hooked() {
  const { compatFor } = useCompat();
  return <LibraryScreen compatFor={compatFor} />;
}

function renderScreen() {
  return render(
    <ToastProvider>
      <LibraryProvider>
        <CompatProvider>
          <main>
            <CompatNotice />
            <Hooked />
          </main>
        </CompatProvider>
      </LibraryProvider>
    </ToastProvider>,
  );
}

slowIt("affiche un badge de compatibilité par carte, avec la raison utile", async () => {
  report = {
    issue: null,
    entries: [
      fiche({ uuid: "ok" }),
      fiche({
        uuid: "avert",
        level: "warnings",
        code: "OK_AVERTISSEMENTS",
        reason: "Exploration partielle (40 %)",
      }),
      fiche({ uuid: "echec", level: "incompatible", code: "ECHEC_VALIDATION" }),
      fiche({ uuid: "futur", level: "incompatible", code: "ECHEC_NOUVEAU", stale: true }),
    ],
  };
  const { container } = renderScreen();
  await screen.findByText("Exploration partielle (40 %)");
  expect(screen.getAllByText("Compatible")).toHaveLength(1);
  expect(screen.getAllByText("Avertissements")).toHaveLength(1);
  expect(screen.getAllByText("Incompatible")).toHaveLength(2);
  expect(screen.getAllByText("Non testée")).toHaveLength(1);
  expect(screen.getByText("L'histoire convertie n'est pas valide.")).toBeInTheDocument();
  expect(
    screen.getByText(/Cette histoire a échoué lors des essais.*Résultat périmé/),
  ).toBeInTheDocument();
  await expectNoAxeViolations(container, PAGE_LEVEL_RULES);
});

it("présente tout comme non testé si le rapport est illisible, avec un bandeau", async () => {
  report = {
    entries: [],
    issue: { kind: "unsupported_version", found: 2, supported: 1 },
  };
  renderScreen();
  expect(await screen.findByText(/format n° 2/)).toBeInTheDocument();
  expect(await screen.findAllByText("Non testée")).toHaveLength(5);
});

it("reste utilisable quand la commande échoue", async () => {
  report = "fail";
  renderScreen();
  expect(await screen.findAllByText("Non testée")).toHaveLength(5);
  expect(screen.queryByText("Rapport de compatibilité indisponible")).toBeNull();
});

it("describeCompat : raison de la fiche, sinon du statut d'échec, sinon du niveau", () => {
  expect(describeCompat(undefined).level).toBe("untested");
  expect(describeCompat(fiche({ uuid: "a", reason: "Dit" })).reason).toBe("Dit");
  expect(
    describeCompat(fiche({ uuid: "a", level: "incompatible", code: "ECHEC_EXECUTION" }))
      .reason,
  ).toMatch(/exécution/);
  expect(
    describeCompat(fiche({ uuid: "a", level: "warnings", code: "?" })).reason,
  ).toMatch(/limites/);
});

slowIt("avertit avant d'installer une histoire incompatible, sans bloquer", async () => {
  const info = describeCompat(
    fiche({ uuid: "a", level: "incompatible", reason: "Image manquante" }),
  );
  expect(needsInstallWarning(info)).toBe(true);
  expect(needsInstallWarning(describeCompat(fiche({ uuid: "a" })))).toBe(false);
  expect(needsInstallWarning(undefined)).toBe(false);
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <IncompatibleInstallDialog
      open
      story="Conte"
      info={info}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />,
  );
  expect(screen.getByRole("alertdialog", { name: /Conte/ })).toBeInTheDocument();
  expect(screen.getByText("Image manquante")).toBeInTheDocument();
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Annuler" })).toHaveFocus(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Installer quand même" }));
  expect(onConfirm).toHaveBeenCalledOnce();
  expect(onCancel).not.toHaveBeenCalled();
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});
