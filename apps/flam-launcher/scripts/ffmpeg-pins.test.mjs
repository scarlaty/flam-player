// Cohérence des sources épinglées de ffmpeg (tools/ffmpeg/pins.env) avec LICENSES.md (#209).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const pins = Object.fromEntries(
  read("../../../tools/ffmpeg/pins.env")
    .split(/\r?\n/)
    .filter((line) => /^[A-Z0-9_]+=/.test(line))
    .map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]),
);

test("chaque source est épinglée par version, URL et SHA-256", () => {
  for (const name of ["FFMPEG", "LAME"]) {
    assert.match(pins[`${name}_SHA256`], /^[0-9a-f]{64}$/);
    assert.ok(pins[`${name}_URL`].startsWith("https://"));
    assert.ok(
      pins[`${name}_URL`].includes(pins[`${name}_VERSION`]),
      `${name} : version absente de l'URL`,
    );
  }
});

test("LICENSES.md cite les versions épinglées de FFmpeg et de LAME", () => {
  const licenses = read("../LICENSES.md");
  assert.ok(licenses.includes(`FFmpeg ${pins.FFMPEG_VERSION}`));
  assert.ok(licenses.includes(`LAME ${pins.LAME_VERSION}`));
});
