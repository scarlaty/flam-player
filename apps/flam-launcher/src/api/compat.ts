import { invokeCommand } from "./error";
import type { StoryEntry } from "./story";

/** Niveau de compatibilité (miroir de `CompatLevel` dans `src-tauri/src/compat.rs`). */
export type CompatLevel = "compatible" | "warnings" | "incompatible" | "untested";

/** Provenance d'une fiche : campagne de compatibilité ou conversion faite par la surcouche. */
export type CompatOrigin = "campaign" | "conversion";

/** Statut d'une histoire (miroir de `CompatEntry`). */
export interface CompatEntry {
  uuid: string;
  /** Version évaluée (`"3"`, `"local"`…), `null` si inconnue. */
  version: string | null;
  level: CompatLevel;
  /** Phrase d'une ligne fournie par la campagne ou la conversion. */
  reason: string | null;
  /** Statut brut de la campagne (`ECHEC_VALIDATION`…) ou code du convertisseur. */
  code: string | null;
  origin: CompatOrigin;
  /** Fiche de campagne évaluée avec un autre convertisseur que le courant. */
  stale: boolean;
  /** Moteur de la conversion faite sur cet ordinateur (#137) ; absent pour une fiche de campagne. */
  engine?: "rust" | "python" | "python_fallback" | null;
  /** Cause du repli Python : `unsupported_format` (attendu), `internal_error` ou `crash`. */
  fallback?: "unsupported_format" | "internal_error" | "crash" | null;
}

/** Problème de lecture de `status.json` (miroir de `CompatIssue`). */
export type CompatIssue =
  | { kind: "unreadable" }
  | { kind: "too_large" }
  | { kind: "unsupported_version"; found: number | null; supported: number };

/** Réponse de `compat_statuses` (miroir de `CompatReport`). */
export interface CompatReport {
  entries: CompatEntry[];
  issue: CompatIssue | null;
}

/** `compat_statuses` : statuts de la campagne et des conversions locales (aucun chemin envoyé). */
export function getCompatReport(): Promise<CompatReport> {
  return invokeCommand<CompatReport>("compat_statuses");
}

/**
 * Une version numérique (`"3"`) doit égaler celle de l'histoire quand elle est connue ;
 * une version inconnue ou textuelle (`"local"`) convient à toute version.
 */
function sameVersion(entryVersion: number | null, compatVersion: string | null): boolean {
  if (entryVersion === null || compatVersion === null) return true;
  if (!/^\d+$/.test(compatVersion)) return true;
  return Number(compatVersion) === entryVersion;
}

/**
 * Fiche de `entry` : même `uuid` et version compatible ; la fiche de version exacte
 * l'emporte. `undefined` : histoire non testée.
 */
export function findCompat(
  entries: readonly CompatEntry[],
  entry: Pick<StoryEntry, "uuid" | "version">,
): CompatEntry | undefined {
  const candidates = entries.filter(
    (c) => c.uuid === entry.uuid && sameVersion(entry.version, c.version),
  );
  const exact = candidates.find(
    (c) => entry.version !== null && c.version === String(entry.version),
  );
  return exact ?? candidates[0];
}
