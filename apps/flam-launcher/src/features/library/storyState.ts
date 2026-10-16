import type { ReactNode } from "react";
import type { StoryEntry } from "../../api/story";
import type { Source } from "../../api/sources";
import type { BadgeTone, IconName } from "../../components";
import type { CompatInfo } from "../compat/compatInfo";

/**
 * État d'une histoire dans la bibliothèque. Les trois premiers découlent de la source ;
 * les autres sont fournis par l'installation, le lancement et le statut de compatibilité
 * via `StoryHooks.resolveState`.
 */
export type StoryState =
  | "to_download"
  | "to_convert"
  | "to_install"
  | "installed"
  | "update_available"
  | "in_progress"
  | "incompatible"
  | "untested";

export const STATE_APPEARANCE: Record<StoryState, { tone: BadgeTone; icon: IconName }> = {
  to_download: { tone: "info", icon: "download" },
  to_convert: { tone: "info", icon: "install" },
  to_install: { tone: "info", icon: "install" },
  installed: { tone: "success", icon: "check" },
  update_available: { tone: "warning", icon: "refresh" },
  in_progress: { tone: "info", icon: "refresh" },
  incompatible: { tone: "danger", icon: "alert" },
  untested: { tone: "neutral", icon: "info" },
};

/** Opération en cours sur une histoire (téléchargement, conversion, installation). */
export interface StoryProgress {
  /** Pourcentage 0-100, ou `"indeterminate"`. */
  value: number | "indeterminate";
  /** Texte visible sous la barre (ex. « Conversion… »). */
  text?: string;
}

/**
 * Points d'accroche des fonctionnalités qui s'ajoutent à l'affichage (installation,
 * lancement, fiche détaillée, compatibilité). Tous sont facultatifs.
 */
export interface StoryHooks {
  /** État de l'histoire ; `undefined` laisse l'état par défaut déduit de la source. */
  resolveState?: (entry: StoryEntry) => StoryState | undefined;
  /** Opération en cours : affiche une barre de progression et l'état « En cours ». */
  progressFor?: (entry: StoryEntry) => StoryProgress | undefined;
  /**
   * Statut de compatibilité de l'histoire : badge supplémentaire (compatible, avertissements,
   * incompatible, non testée) avec sa raison. `undefined` : rien à afficher (chargement).
   */
  compatFor?: (entry: StoryEntry) => CompatInfo | undefined;
  /** Actions rapides d'une carte (ex. Télécharger, Jouer). */
  renderActions?: (entry: StoryEntry) => ReactNode;
  /** Ouvre la fiche de l'histoire (clic sur le titre). */
  onOpenStory?: (entry: StoryEntry) => void;
  /** Histoire sélectionnée (fiche ouverte). */
  selectedEntryId?: string;
}

/** État par défaut, déduit de la nature de la source et de l'archive. */
export function defaultStoryState(source: Source, entry: StoryEntry): StoryState {
  if (source.type === "library") return "installed";
  if (source.type === "catalog") return "to_download";
  return entry.kind === "telmi_zip" ? "to_convert" : "to_install";
}
