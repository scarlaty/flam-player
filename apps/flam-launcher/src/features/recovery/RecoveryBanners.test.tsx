import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { expectNoAxeViolations } from "../../test/axe";
import { RecoveryBanners } from "./RecoveryBanners";
import { useRecovery } from "./RecoveryContext";
import { RecoveryProvider } from "./RecoveryProvider";

afterEach(() => clearMocks());

function Reporter() {
  const { report } = useRecovery();
  return (
    <button
      onClick={() =>
        report("settings.json", {
          file: "settings.json",
          backup: "settings.json.20261015-101010.bak",
        })
      }
    >
      signaler
    </button>
  );
}

function setup() {
  render(
    <RecoveryProvider>
      <Reporter />
      <RecoveryBanners />
    </RecoveryProvider>,
  );
  act(() => screen.getByRole("button", { name: "signaler" }).click());
}

it("n'affiche rien sans restauration", () => {
  render(
    <RecoveryProvider>
      <RecoveryBanners />
    </RecoveryProvider>,
  );
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("affiche le bandeau avec le nom de la sauvegarde puis l'acquitte", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return null;
  });
  setup();

  const banner = screen.getByRole("status");
  expect(banner).toHaveTextContent("Le fichier settings.json était illisible");
  expect(banner).toHaveTextContent("settings.json.20261015-101010.bak");
  await expectNoAxeViolations(document.body, ["region", "page-has-heading-one"]);

  fireEvent.click(screen.getByRole("button", { name: "J'ai compris" }));

  await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
  expect(calls).toEqual([
    { cmd: "config_ack_recovery", args: { file: "settings.json" } },
  ]);
});

it("garde le bandeau et signale l'échec de l'acquittement", async () => {
  mockIPC(() => {
    throw { kind: "io", message: "échec" };
  });
  setup();

  fireEvent.click(screen.getByRole("button", { name: "J'ai compris" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Impossible d'enregistrer l'acquittement.",
  );
  expect(screen.getByRole("status")).toBeInTheDocument();
});
