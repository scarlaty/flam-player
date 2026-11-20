import { expect, it } from "vitest";
import { fitScreen } from "./screenFit";

it("grand espace à 100 % : ×2 au plus (640×480), jamais plus", () => {
  expect(fitScreen(1920, 1000, 1)).toEqual({ width: 640, height: 480 });
  expect(fitScreen(640, 480, 1)).toEqual({ width: 640, height: 480 });
});

it("espace un peu court à 100 % : taille cible non entière, pas de chute à ×1", () => {
  // Cible ≈ 626,7×470 ; ×1 (320) fait moins de 80 % de la cible.
  const size = fitScreen(630, 470, 1);
  expect(size.width).toBeCloseTo((470 * 4) / 3);
  expect(size.height).toBeCloseTo(470);
  expect(fitScreen(639, 1000, 1)).toEqual({ width: 639, height: 479.25 });
});

it("à 100 %, ×1 retenu dès qu'il atteint 80 % de la cible", () => {
  // Cible 400×300 : ×1 (320×240) = 80 %.
  expect(fitScreen(400, 300, 1)).toEqual({ width: 320, height: 240 });
  // Cible 401×300,75 : ×1 ≈ 79,8 %, la cible est gardée.
  expect(fitScreen(401, 1000, 1)).toEqual({ width: 401, height: 300.75 });
});

it("à 150 % : 640×480 px CSS, soit ×3 entier en px physiques", () => {
  expect(fitScreen(1920, 1000, 1.5)).toEqual({ width: 640, height: 480 });
  // Moins de place (cible ≈ 533×400) : ×2 physique (≈ 427×320 px CSS, 80 % de la cible).
  const small = fitScreen(600, 400, 1.5);
  expect(small.width * 1.5).toBeCloseTo(640);
  expect(small.height * 1.5).toBeCloseTo(480);
});

it("à 125 % : ×2 physique (512×384 px CSS, 80 % de la cible) accepté", () => {
  expect(fitScreen(1920, 1000, 1.25)).toEqual({ width: 512, height: 384 });
});

it("arrondi flottant : un facteur entier n'est pas perdu (taille déjà ajustée, 185 %)", () => {
  for (const ratio of [1, 1.15, 1.2, 1.25, 1.5, 1.75, 1.85, 2, 2.75]) {
    const size = fitScreen(1920, 1000, ratio);
    expect(fitScreen(size.width, size.height, ratio)).toEqual(size);
  }
  // 240 × 2 / 1,85 px CSS : ×2 physique, et non ×1 (480 / 1,85 × 1,85 / 240 < 2).
  const size = fitScreen(10_000, (240 * 2) / 1.85, 1.85);
  expect(size.height * 1.85).toBeCloseTo(480);
});

it("si même ×1 ne tient pas : réduit à l'espace en gardant le 4:3", () => {
  const size = fitScreen(160, 1000, 1);
  expect(size).toEqual({ width: 160, height: 120 });
  expect(fitScreen(1000, 60, 1)).toEqual({ width: 80, height: 60 });
});

it("valeurs invalides : espace nul, rapport ramené à 1", () => {
  expect(fitScreen(0, 0, 1)).toEqual({ width: 0, height: 0 });
  expect(fitScreen(-5, Number.NaN, 1)).toEqual({ width: 0, height: 0 });
  expect(fitScreen(1920, 1000, 0)).toEqual({ width: 640, height: 480 });
  expect(fitScreen(1920, 1000, Number.NaN)).toEqual({ width: 640, height: 480 });
});
