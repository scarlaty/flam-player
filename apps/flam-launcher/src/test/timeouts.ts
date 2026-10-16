import { configure, getConfig } from "@testing-library/react";
import { afterAll, beforeAll, it } from "vitest";

/**
 * Délais des tests lents sous charge CPU (issue #94). Le `testTimeout` global (5 s) reste
 * inchangé : seuls les tests qui rendent un écran entier puis lancent axe-core (plusieurs
 * secondes quand la machine est saturée) reçoivent ce délai, en second argument de `it`.
 */
export const SLOW_TEST_MS = 15_000;

/** Délai des `findBy…` et `waitFor` d'un fichier de tests lents (défaut de Testing Library : 1 s). Reste sous le `testTimeout` de 5 s pour qu'un échec réel garde le message de Testing Library.. */
export const ASYNC_UTIL_MS = 4_000;

/** Relève `asyncUtilTimeout` le temps d'un fichier de tests, puis le rétablit. */
export function tolerateLoadInAsyncUtils(): void {
  let previous = 0;
  beforeAll(() => {
    previous = getConfig().asyncUtilTimeout;
    configure({ asyncUtilTimeout: ASYNC_UTIL_MS });
  });
  afterAll(() => {
    configure({ asyncUtilTimeout: previous });
  });
}

/** `it` avec le délai des tests lents (voir `SLOW_TEST_MS`). */
export function slowIt(name: string, fn: () => Promise<void>): void {
  it(name, fn, SLOW_TEST_MS);
}
