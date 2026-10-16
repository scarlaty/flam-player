import { expect, it } from "vitest";
import {
  formatBytes,
  validateCatalogUrl,
  validateName,
  validateOptionalPath,
} from "./validation";

it("valide le nom", () => {
  expect(validateName("Mes contes")).toBeNull();
  expect(validateName("   ")).toMatch(/vide/);
  expect(validateName("a\nb")).toMatch(/contrôle/);
  expect(validateName("é".repeat(100))).toBeNull();
  expect(validateName("é".repeat(101))).toMatch(/100/);
});

it("valide l'URL du catalogue", () => {
  expect(validateCatalogUrl("https://exemple.org/c.json")).toBeNull();
  expect(validateCatalogUrl(" http://exemple.org ")).toBeNull();
  expect(validateCatalogUrl("")).toMatch(/vide/);
  expect(validateCatalogUrl("exemple.org")).toMatch(/http/);
  expect(validateCatalogUrl("ftp://exemple.org/c.json")).toMatch(/http/);
  expect(validateCatalogUrl("https://user:pw@exemple.org/")).toMatch(/identifiants/);
  expect(validateCatalogUrl(`https://exemple.org/${"a".repeat(2048)}`)).toMatch(/2048/);
});

it("valide les chemins facultatifs", () => {
  expect(validateOptionalPath("", { exe: true })).toBeNull();
  expect(validateOptionalPath("C:\\app\\flam-player.EXE", { exe: true })).toBeNull();
  expect(validateOptionalPath("C:\\app\\run.bat", { exe: true })).toMatch(/\.exe/);
  expect(validateOptionalPath("app\\p.exe", { exe: true })).toMatch(/complet/);
  expect(validateOptionalPath("\\\\srv\\p\\p.exe", { exe: true })).toMatch(/réseau/);
  expect(validateOptionalPath("D:/cache", { exe: false })).toBeNull();
});

it("formate les tailles en français", () => {
  expect(formatBytes(0)).toBe("0 o");
  expect(formatBytes(1023)).toBe("1023 o");
  expect(formatBytes(1536)).toMatch(/^1,5 Ko$/);
  expect(formatBytes(5 * 1024 * 1024)).toBe("5 Mo");
});
