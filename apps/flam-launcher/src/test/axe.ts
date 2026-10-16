import axe from "axe-core";
import { expect } from "vitest";

/** Règles propres à une page entière, sans objet pour un composant isolé. */
export const PAGE_LEVEL_RULES = ["region", "landmark-one-main", "page-has-heading-one"];

/**
 * Vérifie qu'axe ne signale aucune violation dans `context`.
 * `color-contrast` est toujours désactivée ici : jsdom ne calcule pas les styles
 * (les contrastes sont vérifiés dans un vrai navigateur, voir docs/DESIGN.md).
 */
export async function expectNoAxeViolations(
  context: Element = document.body,
  disabledRules: string[] = [],
): Promise<void> {
  const rules = Object.fromEntries(
    ["color-contrast", ...disabledRules].map((id) => [id, { enabled: false }]),
  );
  const results = await axe.run(context, { rules });
  const summary = results.violations.map(
    (violation) =>
      `${violation.id} : ${violation.help} → ${violation.nodes
        .map((node) => node.target.join(" "))
        .join(", ")}`,
  );
  expect(summary).toEqual([]);
}

/** Variante pour un composant rendu seul (sans structure de page). */
export function expectComponentAccessible(container: Element): Promise<void> {
  return expectNoAxeViolations(container, PAGE_LEVEL_RULES);
}
