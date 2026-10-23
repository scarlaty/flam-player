import { expect, it } from "vitest";
import { libraryHash, routeFromHash, sourceIdFromHash } from "./router";

it("route les hash de la bibliothèque", () => {
  expect(routeFromHash("#/library")).toBe("library");
  expect(routeFromHash("#/library/telmi-store")).toBe("library");
  expect(routeFromHash("#/libraryx")).toBe("home");
});

it("encode et décode l'identifiant de source", () => {
  expect(libraryHash()).toBe("#/library");
  expect(libraryHash("a b/é")).toBe("#/library/a%20b%2F%C3%A9");
  expect(sourceIdFromHash(libraryHash("a b/é"))).toBe("a b/é");
  expect(sourceIdFromHash("#/library")).toBeNull();
  expect(sourceIdFromHash("#/library/")).toBeNull();
  expect(sourceIdFromHash("#/library/%E0%A4%A")).toBeNull();
  expect(sourceIdFromHash("#/sources")).toBeNull();
});

it("route l'écran Ma Flam", () => {
  expect(routeFromHash("#/device")).toBe("device");
  expect(routeFromHash("#/devices")).toBe("home");
});

it("route l'éditeur de pack sous les podcasts", () => {
  expect(routeFromHash("#/podcasts")).toBe("podcasts");
  expect(routeFromHash("#/podcasts/pack")).toBe("pack");
  expect(routeFromHash("#/podcasts/packs")).toBe("home");
});
