/**
 * Texte d'une table par code ou par `kind`, seulement si la clé est propre à la table : un code
 * libre (`constructor`, `__proto__`, `toString`…) n'est jamais pris pour une clé héritée.
 */
export function ownText(
  table: Readonly<Record<string, string>>,
  key: string,
): string | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined;
}
