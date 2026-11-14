import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import { t } from "../../i18n";
import { expectNoAxeViolations } from "../../test/axe";
import { tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { CloseRetainedBanner } from "./CloseRetainedBanner";

tolerateLoadInAsyncUtils();

let calls: string[];
/** Opération que `device_close_retained` rend (`null` : aucune retenue). */
let retained: string | null;

beforeEach(() => {
  calls = [];
  retained = null;
  mockIPC(
    (cmd) => {
      calls.push(cmd);
      if (cmd === "device_close_retained") return retained;
      return cmd === "device_cancel_close" ? true : null;
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  cleanup();
  clearMocks();
});

async function retain(operation: string) {
  // L'abonnement est asynchrone : l'événement n'est reçu qu'une fois posé.
  await waitFor(async () => {
    await act(async () => {
      await emit("device://close-retained", { operation });
    });
    expect(screen.queryByRole("status")).not.toBeNull();
  });
}

it("n'affiche rien sans retenue", async () => {
  render(<CloseRetainedBanner />);
  expect(screen.queryByRole("status")).toBeNull();
});

it("annonce la retenue avec ce que fait la reprise de l'opération", async () => {
  for (const operation of ["transfer", "remove", "reorder"] as const) {
    const { unmount } = render(<CloseRetainedBanner />);
    await retain(operation);
    const banner = screen.getByRole("status");
    expect(banner).toHaveTextContent(t.closeRetained.title);
    expect(banner).toHaveTextContent(t.closeRetained.body);
    expect(banner).toHaveTextContent(t.closeRetained.resume[operation]);
    expect(banner).toHaveTextContent(
      "La fenêtre se fermera à la fin de l'opération en cours",
    );
    expect(banner).toHaveTextContent("Les envois en attente ne partiront pas");
    expect(banner).toHaveTextContent("Fermez à nouveau pour interrompre maintenant");
    if (operation === "transfer") await expectNoAxeViolations(document.body);
    unmount();
  }
});

it("disparaît quand la retenue prend fin sans fermeture", async () => {
  render(<CloseRetainedBanner />);
  await retain("transfer");
  await act(async () => {
    await emit("device://close-cleared");
  });
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  expect(calls).not.toContain("device_cancel_close");
});

it("« Ne pas fermer » lève la retenue côté backend et retire le bandeau", async () => {
  render(<CloseRetainedBanner />);
  await retain("remove");
  fireEvent.click(screen.getByRole("button", { name: t.closeRetained.keep }));
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  expect(calls).toContain("device_cancel_close");
});

it("retire le bandeau même si la levée de la retenue échoue", async () => {
  mockIPC(
    (cmd) => {
      if (cmd === "device_cancel_close") throw { kind: "io", message: "boum" };
      return null;
    },
    { shouldMockEvents: true },
  );
  render(<CloseRetainedBanner />);
  await retain("reorder");
  fireEvent.click(screen.getByRole("button", { name: t.closeRetained.keep }));
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
});

it("rattrape une retenue déjà en cours au montage (événement manqué)", async () => {
  retained = "remove";
  render(<CloseRetainedBanner />);
  const banner = await screen.findByRole("status");
  expect(banner).toHaveTextContent(t.closeRetained.resume.remove);
});
