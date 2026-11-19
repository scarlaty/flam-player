import { act, cleanup, render, waitFor } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it, vi } from "vitest";
import { EDITOR_RELOCATED_EVENT, EDITOR_TASK_EVENT } from "../../../api/editor";
import { StoriesContext, type StoriesApi } from "../../stories/StoriesContext";
import { EditorTaskBridge } from "./EditorTaskBridge";

afterEach(() => {
  cleanup();
  clearMocks();
});

it("fenêtre principale : tâches suivies et Mes histoires relu après un changement de dossier", async () => {
  mockIPC(() => null, { shouldMockEvents: true });
  const trackTask = vi.fn();
  const refreshInstalled = vi.fn(() => Promise.resolve());
  const api = { trackTask, refreshInstalled } as unknown as StoriesApi;
  render(
    <StoriesContext.Provider value={api}>
      <EditorTaskBridge />
    </StoriesContext.Provider>,
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  await act(() => emit(EDITOR_RELOCATED_EVENT));
  await waitFor(() => expect(refreshInstalled).toHaveBeenCalledTimes(1));
  await act(() => emit(EDITOR_TASK_EVENT, { id: "t" }));
  await waitFor(() => expect(trackTask).toHaveBeenCalledWith({ id: "t" }));
});
