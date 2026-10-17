import assert from "node:assert/strict";
import { test } from "node:test";
import {
  isAllowedLicense,
  optionalMissingIsExpected,
  platformExcluded,
  thirdPartyRustIds,
} from "./notices-lib.mjs";

test("platformExcluded : liste blanche", () => {
  assert.equal(platformExcluded(["linux"], "win32"), true);
  assert.equal(platformExcluded(["linux", "win32"], "win32"), false);
  assert.equal(platformExcluded("darwin", "win32"), true);
});

test("platformExcluded : négation", () => {
  assert.equal(platformExcluded(["!win32"], "win32"), true);
  assert.equal(platformExcluded(["!linux"], "win32"), false);
  assert.equal(platformExcluded(["linux", "!win32"], "win32"), true);
});

test("platformExcluded : champ absent ou vide", () => {
  assert.equal(platformExcluded(undefined, "win32"), false);
  assert.equal(platformExcluded([], "win32"), false);
});

test("optionnel absent : toléré seulement si os ou cpu exclut la plateforme", () => {
  const e = (x) => optionalMissingIsExpected(x, "win32", "x64");
  assert.equal(e({ optional: true, os: ["linux"], cpu: ["x64"] }), true);
  assert.equal(e({ optional: true, os: ["win32"], cpu: ["arm64"] }), true);
  assert.equal(e({ optional: true, os: ["win32"], cpu: ["x64"] }), false);
  assert.equal(e({ optional: true }), false);
  assert.equal(e({ os: ["linux"] }), false);
});

test("licences autorisées", () => {
  for (const l of [
    "MIT",
    "MIT OR Apache-2.0",
    "MIT/Apache-2.0",
    "(MIT OR Apache-2.0) AND Unicode-3.0",
    "Apache-2.0 WITH LLVM-exception",
    "Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT",
  ])
    assert.equal(isAllowedLicense(l), true, l);
});

test("licences refusées", () => {
  for (const l of [
    "",
    undefined,
    "GPL-3.0",
    "MIT OR GPL-3.0",
    "MIT AND",
    "(MIT",
    "MIT WITH Classpath-exception-2.0",
    "Apache-2.0 WITH",
    "voir LICENSE.txt",
  ])
    assert.equal(isAllowedLicense(l), false, String(l));
});

test("membres du workspace exclus des paquets Rust tiers", () => {
  const reached = new Set(["launcher", "telmi2flam", "serde", "zip"]);
  const ids = thirdPartyRustIds(reached, "launcher", ["launcher", "telmi2flam"]);
  assert.deepEqual([...ids].sort(), ["serde", "zip"]);
  // Sans liste de membres : seule la racine est retirée.
  assert.deepEqual([...thirdPartyRustIds(reached, "launcher", undefined)].sort(), [
    "serde",
    "telmi2flam",
    "zip",
  ]);
});
