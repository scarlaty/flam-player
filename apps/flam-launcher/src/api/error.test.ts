import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import { invokeCommand, isAppError } from "./error";

afterEach(() => clearMocks());

describe("isAppError", () => {
  it("reconnaît la forme { kind, message }", () => {
    expect(isAppError({ kind: "io", message: "x" })).toBe(true);
  });

  it("rejette les autres formes", () => {
    expect(isAppError("texte")).toBe(false);
    expect(isAppError(null)).toBe(false);
    expect(isAppError({ kind: "io" })).toBe(false);
    expect(isAppError({ kind: 1, message: "x" })).toBe(false);
  });
});

describe("invokeCommand", () => {
  it("renvoie la valeur en cas de succès", async () => {
    mockIPC(() => 42);
    await expect(invokeCommand<number>("x")).resolves.toBe(42);
  });

  it("conserve une AppError du backend", async () => {
    mockIPC(() => {
      throw { kind: "io", message: "boum" };
    });
    await expect(invokeCommand("x")).rejects.toEqual({ kind: "io", message: "boum" });
  });

  it("normalise une chaîne en kind unknown", async () => {
    mockIPC(() => {
      throw "oups";
    });
    await expect(invokeCommand("x")).rejects.toEqual({
      kind: "unknown",
      message: "oups",
    });
  });

  it("normalise une forme inattendue en kind unknown", async () => {
    mockIPC(() => {
      throw { code: 12 };
    });
    await expect(invokeCommand("x")).rejects.toEqual({
      kind: "unknown",
      message: "Erreur inconnue",
    });
  });
});
