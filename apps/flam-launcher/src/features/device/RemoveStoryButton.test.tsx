import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import type { DeviceStory } from "../../api/device";
import { ToastProvider } from "../../components";
import { RemoveStoryButton } from "./RemoveStoryButton";

const removeFromDevice = vi.fn(() => Promise.resolve({ cleanupPending: false }));

vi.mock("./DeviceContext", () => ({
  useDevice: () => ({
    removeFromDevice,
    deviceKey: "cle",
    ejecting: false,
    canMaintain: true,
  }),
}));

const story = { uuid: "a".repeat(32) } as unknown as DeviceStory;

function ui(name: string) {
  return (
    <ToastProvider>
      <RemoveStoryButton story={story} name={name} busy={false} onRemoved={() => {}} />
    </ToastProvider>
  );
}

it("fige le nom à l'ouverture du dialogue, question et notification comprises (#581)", async () => {
  const { rerender } = render(ui("Histoire aaaaaaaa"));
  fireEvent.click(screen.getByRole("button", { name: /^Retirer « Histoire aaaaaaaa »/ }));
  const dialog = await screen.findByRole("alertdialog", {
    name: "Retirer « Histoire aaaaaaaa » de la Flam ?",
  });

  // Le titre déchiffré arrive pendant que le dialogue est ouvert.
  rerender(ui("Capi et la rivière"));
  expect(dialog).toHaveAccessibleName("Retirer « Histoire aaaaaaaa » de la Flam ?");

  fireEvent.click(screen.getByRole("button", { name: "Retirer de la Flam" }));
  expect(
    await screen.findByText("« Histoire aaaaaaaa » retirée de la Flam."),
  ).toBeVisible();
  await waitFor(() => expect(removeFromDevice).toHaveBeenCalledOnce());
});
