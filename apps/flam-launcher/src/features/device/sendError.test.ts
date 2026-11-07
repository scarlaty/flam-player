import { expect, it } from "vitest";
import { LUNII_NEEDS_V2 } from "../../api/device";
import { sendErrorMessage } from "./sendError";

it("histoire STUdio sur une Flam 1.x : message du front, pas celui du backend", () => {
  const message = sendErrorMessage({ kind: LUNII_NEEDS_V2, message: "texte du backend" });
  expect(message).toBe(
    "Les histoires STUdio demandent une Flam en firmware 2.x : cette Flam ne peut pas les lire. Rien n'a été écrit sur la Flam.",
  );
  expect(LUNII_NEEDS_V2).toBe("lunii_needs_v2");
});

it("autres erreurs : message du backend tel quel", () => {
  expect(sendErrorMessage({ kind: "validation", message: "espace insuffisant" })).toBe(
    "espace insuffisant",
  );
  expect(sendErrorMessage({ kind: "unknown", message: "boum" })).toBe("boum");
});
