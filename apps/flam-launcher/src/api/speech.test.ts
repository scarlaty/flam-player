import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import {
  effectiveVoice,
  listVoices,
  previewSpeech,
  stopSpeechPreview,
  type VoiceList,
} from "./speech";

afterEach(() => clearMocks());

const list: VoiceList = {
  voices: [
    { id: "a", name: "A", language: "fr-FR" },
    { id: "b", name: "B", language: "fr-CA" },
  ],
  default_id: "b",
};

it("voix effective : voix voulue si installée, sinon voix par défaut", () => {
  expect(effectiveVoice(list, "a")).toBe("a");
  expect(effectiveVoice(list, "disparue")).toBe("b");
  expect(effectiveVoice(list, null)).toBe("b");
  expect(effectiveVoice({ ...list, default_id: null }, null)).toBe("a");
  expect(effectiveVoice({ voices: [], default_id: null }, "a")).toBeNull();
});

it("appelle les commandes de synthèse avec leurs arguments", async () => {
  const seen: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    seen.push([cmd, args]);
    if (cmd === "tts_voices") return list;
    if (cmd === "tts_preview") return { played: true, duration_ms: 1200 };
    return null;
  });
  expect(await listVoices()).toEqual(list);
  expect(await previewSpeech("Bonjour", "a")).toEqual({
    played: true,
    duration_ms: 1200,
  });
  await previewSpeech("Bonjour");
  await stopSpeechPreview();
  expect(seen).toEqual([
    ["tts_voices", {}],
    ["tts_preview", { text: "Bonjour", voice: "a" }],
    ["tts_preview", { text: "Bonjour", voice: null }],
    ["tts_preview_stop", {}],
  ]);
});
