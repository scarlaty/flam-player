/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, renderHook, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { previewErrorMessage, useSpeechPreview } from "./speech";

type Info = { played: boolean; duration_ms: number };

let calls: { cmd: string; args: Record<string, any> }[];
let replies: ((info: Info) => void)[];

beforeEach(() => {
  calls = [];
  replies = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, any> });
    switch (cmd) {
      case "tts_preview":
        return new Promise((resolve) => replies.push(resolve));
      case "tts_preview_stop":
        return null;
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => {
  vi.useRealTimers();
  clearMocks();
});

const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);

it("une réponse ancienne après un nouveau play est ignorée", async () => {
  const { result } = renderHook(() => useSpeechPreview());
  act(() => result.current.play("un", null));
  act(() => result.current.play("deux", null));
  await waitFor(() => expect(replies).toHaveLength(2));

  // La première réponse (périmée) n'arme aucun minuteur et ne change pas l'état.
  await act(async () => replies[0]!({ played: true, duration_ms: 10 }));
  expect(result.current.playing).toBe(true);
  await new Promise((r) => setTimeout(r, 40));
  expect(result.current.playing).toBe(true);

  await act(async () => replies[1]!({ played: false, duration_ms: 0 }));
  expect(result.current.playing).toBe(false);
});

it("une réponse ancienne après stop est ignorée", async () => {
  const { result } = renderHook(() => useSpeechPreview());
  act(() => result.current.play("un", null));
  await waitFor(() => expect(replies).toHaveLength(1));
  act(() => result.current.stop());
  expect(result.current.playing).toBe(false);
  await waitFor(() => expect(callsOf("tts_preview_stop")).toHaveLength(1));

  await act(async () => replies[0]!({ played: true, duration_ms: 10 }));
  await new Promise((r) => setTimeout(r, 40));
  expect(result.current.playing).toBe(false);
  expect(result.current.error).toBeNull();
});

it("le minuteur de fin de son est nettoyé par stop, par un nouveau play et au démontage", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const { result, unmount } = renderHook(() => useSpeechPreview());

  act(() => result.current.play("un", null));
  await vi.waitFor(() => expect(replies).toHaveLength(1));
  await act(async () => replies[0]!({ played: true, duration_ms: 5_000 }));
  expect(vi.getTimerCount()).toBe(1);
  act(() => result.current.stop());
  expect(vi.getTimerCount()).toBe(0);
  expect(result.current.playing).toBe(false);

  act(() => result.current.play("deux", null));
  await vi.waitFor(() => expect(replies).toHaveLength(2));
  await act(async () => replies[1]!({ played: true, duration_ms: 5_000 }));
  expect(vi.getTimerCount()).toBe(1);
  // Un nouveau play remplace le minuteur au lieu de l'empiler.
  act(() => result.current.play("trois", null));
  expect(vi.getTimerCount()).toBe(0);
  await vi.waitFor(() => expect(replies).toHaveLength(3));
  await act(async () => replies[2]!({ played: true, duration_ms: 5_000 }));
  expect(vi.getTimerCount()).toBe(1);

  // Fin naturelle du son : retour à l'état de repos.
  act(() => {
    vi.advanceTimersByTime(5_000);
  });
  expect(result.current.playing).toBe(false);
  expect(vi.getTimerCount()).toBe(0);

  act(() => result.current.play("quatre", null));
  await vi.waitFor(() => expect(replies).toHaveLength(4));
  await act(async () => replies[3]!({ played: true, duration_ms: 5_000 }));
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
  expect(callsOf("tts_preview_stop").length).toBeGreaterThan(0);
});

it("double montage StrictMode : rien coupé à vide, aperçu coupé au démontage", async () => {
  const { result, unmount } = renderHook(() => useSpeechPreview(), {
    wrapper: StrictMode,
  });
  expect(result.current.playing).toBe(false);
  // Le démontage simulé par StrictMode n'a rien coupé : aucun aperçu n'était actif.
  expect(callsOf("tts_preview_stop")).toHaveLength(0);
  act(() => result.current.play("un", null));
  await waitFor(() => expect(replies).toHaveLength(1));
  await act(async () => replies[0]!({ played: true, duration_ms: 60_000 }));
  expect(result.current.playing).toBe(true);
  unmount();
  await waitFor(() => expect(callsOf("tts_preview_stop")).toHaveLength(1));
});

it("previewErrorMessage : table par code, jamais de clé héritée", () => {
  const error = (kind: string) => ({ kind, message: "message du backend" });
  expect(previewErrorMessage(error("tts_failed"))).toBe(
    "La voix n'a pas pu lire ce texte. Réessayez.",
  );
  for (const kind of ["__proto__", "constructor", "toString", "hasOwnProperty"]) {
    expect(previewErrorMessage(error(kind))).toBe("message du backend");
  }
});
