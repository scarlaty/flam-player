// Fonctions pures de build-notices.mjs (décision sur les paquets optionnels, liste d'autorisation
// SPDX), isolées pour être testées avec `node --test` (voir notices-lib.test.mjs).

// Licences acceptées pour les composants tiers embarqués. Toute autre licence fait échouer
// `npm run build:notices` : l'ajout d'une licence est une décision du mainteneur.
export const ALLOWED_LICENSES = new Set([
  "MIT",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "MPL-2.0",
  "CC0-1.0",
  "0BSD",
  "BSL-1.0",
  // À valider par le mainteneur (rencontrées dans les dépendances Rust, hors liste initiale) :
  "Unlicense",
  "MIT-0",
  "CDLA-Permissive-2.0",
]);
export const ALLOWED_EXCEPTIONS = new Set(["LLVM-exception"]);

// Évalue une expression SPDX : chaque terme (identifiant, éventuellement `WITH exception`) doit
// être autorisé, quel que soit l'opérateur (`OR`, `AND`, ou l'ancien `/` de Cargo).
export function isAllowedLicense(expression) {
  const tokens = String(expression ?? "")
    .replace(/\(/g, " ( ")
    .replace(/\)/g, " ) ")
    .replace(/\//g, " OR ")
    .split(/\s+/)
    .filter(Boolean);
  if (tokens.length === 0) return false;
  let i = 0;
  const parseExpr = () => {
    let ok = parseTerm();
    while (tokens[i] === "OR" || tokens[i] === "AND") {
      i++;
      ok = parseTerm() && ok;
    }
    return ok;
  };
  const parseTerm = () => {
    const t = tokens[i++];
    if (t === undefined) return false;
    if (t === "(") {
      const ok = parseExpr();
      if (tokens[i++] !== ")") return false;
      return ok;
    }
    if (t === ")" || t === "OR" || t === "AND" || t === "WITH") return false;
    let ok = ALLOWED_LICENSES.has(t);
    if (tokens[i] === "WITH") {
      i++;
      const ex = tokens[i++];
      ok = ok && ex !== undefined && ALLOWED_EXCEPTIONS.has(ex);
    }
    return ok;
  };
  const ok = parseExpr();
  return ok && i === tokens.length;
}

// Sémantique npm (npm-install-checks) d'un champ `os` ou `cpu` : liste blanche, et `!x` exclut x.
// Champ absent ou vide : aucune restriction.
export function platformExcluded(list, value) {
  if (list === undefined || list === null) return false;
  const items = Array.isArray(list) ? list : [list];
  if (items.length === 0) return false;
  if (items.includes(`!${value}`)) return true;
  const allowed = items.filter((x) => !x.startsWith("!"));
  return allowed.length > 0 && !allowed.includes(value);
}

// Un paquet optionnel absent du disque n'est toléré que si son `os` ou son `cpu` exclut la
// plateforme courante ; sinon l'installation est incomplète.
export function optionalMissingIsExpected(entry, platform, arch) {
  return Boolean(
    entry.optional &&
    (platformExcluded(entry.os, platform) || platformExcluded(entry.cpu, arch)),
  );
}
