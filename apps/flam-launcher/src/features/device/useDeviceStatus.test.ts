import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeviceStatus } from "../../api/device";
import { getDeviceStatus } from "../../api/device";
import { useDeviceStatus } from "./useDeviceStatus";

vi.mock("../../api/device", () => ({ getDeviceStatus: vi.fn() }));

const mocked = vi.mocked(getDeviceStatus);

beforeEach(() => {
  mocked.mockReset();
  mocked.mockResolvedValue({ state: "absent" } as unknown as DeviceStatus);
});

afterEach(() => vi.restoreAllMocks());

it("fenêtre masquée : « Réessayer » est close sans appel, avec le numéro rendu", async () => {
  vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  const { result } = renderHook(() => useDeviceStatus());
  // Le premier tour (masqué) ne fait aucun appel.
  await waitFor(() => expect(result.current.answered).toBe(0));
  let request = 0;
  act(() => {
    request = result.current.retry();
  });
  expect(request).toBe(1);
  await waitFor(() => expect(result.current.answered).toBe(request));
  act(() => {
    request = result.current.retry();
  });
  expect(request).toBe(2);
  await waitFor(() => expect(result.current.answered).toBe(2));
  expect(mocked).not.toHaveBeenCalled();
  expect(result.current.status).toBeNull();
});

it("scrutation désactivée : « Réessayer » est close aussitôt, sans appel", () => {
  const { result } = renderHook(() => useDeviceStatus({ enabled: false }));
  let request = 0;
  act(() => {
    request = result.current.retry();
  });
  expect(request).toBe(1);
  expect(result.current.answered).toBe(1);
  act(() => {
    request = result.current.retry();
  });
  expect(request).toBe(2);
  expect(result.current.answered).toBe(2);
  expect(mocked).not.toHaveBeenCalled();
});

it("scrutation mise en pause : « Réessayer » ne lance plus aucune lecture", async () => {
  const { result, rerender } = renderHook(
    ({ enabled }: { enabled: boolean }) =>
      useDeviceStatus({ enabled, intervalMs: 60_000 }),
    { initialProps: { enabled: true } },
  );
  await waitFor(() => expect(result.current.status).not.toBeNull());
  const before = mocked.mock.calls.length;
  rerender({ enabled: false });
  let request = 0;
  act(() => {
    request = result.current.retry();
  });
  // La demande est close aussitôt, sans lecture de la Flam (pause : lecture incrustée).
  expect(result.current.answered).toBe(request);
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(mocked.mock.calls.length).toBe(before);
});
