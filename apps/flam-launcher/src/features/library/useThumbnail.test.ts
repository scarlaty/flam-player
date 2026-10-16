import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import type { Source } from "../../api/sources";
import { makeEntry } from "../../test/entries";
import { useThumbnail } from "./useThumbnail";

const library: Source = { type: "library", id: "library", name: "Mes histoires" };
const FIRST = "data:image/png;base64,AAAA";
const SECOND = "data:image/png;base64,BBBB";

/** Réponses de `library_thumbnail`, servies dans l'ordre et résolues à la demande. */
let pending: ((url: string) => void)[];

beforeEach(() => {
  pending = [];
  mockIPC((cmd) => {
    if (cmd === "library_thumbnail") {
      return new Promise<string>((resolve) => pending.push(resolve));
    }
    throw { kind: "unknown", message: cmd };
  });
});

afterEach(() => {
  clearMocks();
});

it("réinstallation avec une nouvelle vignette : image rafraîchie sans remontage (#97)", async () => {
  const before = { ...makeEntry({ title: "ours" }), library_thumb: "ours-1.1.png" };
  const { result, rerender } = renderHook(({ entry }) => useThumbnail(library, entry), {
    initialProps: { entry: before },
  });
  await waitFor(() => expect(pending).toHaveLength(1));
  await act(async () => pending[0]?.(FIRST));
  expect(result.current.url).toBe(FIRST);

  // Même histoire, même carte (pas de remontage), vignette conservée d'une nouvelle installation.
  rerender({ entry: { ...before, library_thumb: "ours-1.2.png" } });
  // L'ancienne image n'est plus attribuée à la nouvelle installation.
  expect(result.current.url).toBeUndefined();
  await waitFor(() => expect(pending).toHaveLength(2));
  await act(async () => pending[1]?.(SECOND));
  expect(result.current.url).toBe(SECOND);
});
