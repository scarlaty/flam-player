import type { CompatEntry, CompatLevel } from "../../api/compat";
import type { BadgeTone, IconName, StoryBadge } from "../../components";
import { t } from "../../i18n";

/** Statut de compatibilité prêt à afficher. */
export interface CompatInfo {
  level: CompatLevel;
  /** Raison lisible (jamais vide). */
  reason: string;
  /** Résultat évalué avec un autre convertisseur que le courant. */
  stale: boolean;
}

export const COMPAT_APPEARANCE: Record<CompatLevel, { tone: BadgeTone; icon: IconName }> =
  {
    compatible: { tone: "success", icon: "check" },
    warnings: { tone: "warning", icon: "alert" },
    incompatible: { tone: "danger", icon: "alert" },
    untested: { tone: "neutral", icon: "info" },
  };

/**
 * Statut affiché d'une fiche ; `undefined` (histoire absente du rapport) : non testée.
 * La raison est celle de la fiche, sinon une phrase par défaut selon le statut d'échec ou
 * le niveau.
 */
export function describeCompat(entry: CompatEntry | undefined): CompatInfo {
  if (!entry) {
    return { level: "untested", reason: t.compat.defaultReason.untested, stale: false };
  }
  const byCode = entry.code ? t.compat.failureReason[entry.code] : undefined;
  return {
    level: entry.level,
    reason: entry.reason ?? byCode ?? t.compat.defaultReason[entry.level],
    stale: entry.stale,
  };
}

/** Raison suivie de la mention « périmé » le cas échéant. */
export function reasonText(info: CompatInfo): string {
  return info.stale ? `${info.reason} ${t.compat.stale}` : info.reason;
}

/**
 * Badge d'une carte. La raison est affichée sous le badge pour les avertissements et les
 * incompatibilités ; « compatible » et « non testée » restent sobres.
 */
export function compatBadge(info: CompatInfo): StoryBadge {
  return {
    id: `compat-${info.level}`,
    label: t.compat.level[info.level],
    ...COMPAT_APPEARANCE[info.level],
    detail:
      info.level === "warnings" || info.level === "incompatible"
        ? reasonText(info)
        : undefined,
  };
}
