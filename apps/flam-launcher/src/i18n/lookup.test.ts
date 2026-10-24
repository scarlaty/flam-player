import { expect, it } from "vitest";
import { t } from ".";
import { ownText } from "./lookup";

const INHERITED = ["constructor", "__proto__", "toString", "hasOwnProperty", "valueOf"];

it("ownText ne renvoie que les clés propres à la table", () => {
  const table = { a: "A" };
  expect(ownText(table, "a")).toBe("A");
  expect(ownText(table, "b")).toBeUndefined();
  for (const key of INHERITED) expect(ownText(table, key)).toBeUndefined();
});

it("les tables par code ignorent les clés héritées", () => {
  for (const key of INHERITED) {
    expect(t.errors.title(key)).toBe("Une erreur est survenue");
    expect(ownText(t.errors.task, key)).toBeUndefined();
    expect(ownText(t.errors.conversion, key)).toBeUndefined();
    expect(ownText(t.speech.preview, key)).toBeUndefined();
  }
  expect(t.errors.title("tts_failed")).toBe("Titres non lus à voix haute");
});
