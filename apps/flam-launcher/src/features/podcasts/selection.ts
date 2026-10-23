import type { Episode } from "../../api/podcast";

/** Plafonds fixes par pack (D1). 1 Mo = 10⁶ octets ; 1 Gio = 2³⁰ octets. */
export const MAX_EPISODES = 50;
export const MAX_EPISODE_BYTES = 200_000_000;
export const MAX_PACK_BYTES = 1_073_741_824;

/** Débit supposé quand la taille n'est pas annoncée : 192 kbit/s, le cas le plus coûteux (D5, D6). */
const FALLBACK_BYTES_PER_SECOND = 24_000;

/**
 * Raccourci « N derniers épisodes » : au choix du mainteneur (spec § 9, #212). Désactivé tant
 * qu'il n'a pas tranché ; la fonction `latestIds` reste prête.
 */
export const LATEST_SHORTCUT_ENABLED = false;

/** Taille d'un épisode : annoncée par l'enclosure, sinon durée × débit (D5), sinon inconnue. */
export interface SizeEstimate {
  bytes: number | null;
  /** `true` : estimée à partir de la durée (affichée « environ »). */
  approximate: boolean;
}

export function estimateSize(episode: Pick<Episode, "enclosure_length" | "duration">) {
  if (episode.enclosure_length !== null && episode.enclosure_length > 0) {
    return { bytes: episode.enclosure_length, approximate: false } satisfies SizeEstimate;
  }
  if (episode.duration !== null && episode.duration > 0) {
    return {
      bytes: episode.duration * FALLBACK_BYTES_PER_SECOND,
      approximate: true,
    } satisfies SizeEstimate;
  }
  return { bytes: null, approximate: true } satisfies SizeEstimate;
}

/** Épisode retenu pour le pack (instantané : reste valable si le flux est rafraîchi). */
export interface SelectedEpisode {
  id: string;
  sourceId: string;
  sourceName: string;
  title: string;
  /** Taille estimée en octets ; `null` : inconnue (comptée pour 0 dans le total). */
  bytes: number | null;
  explicit: boolean;
}

export type Refusal = "count" | "episodeTooBig" | "total" | "duplicate";

export interface Totals {
  count: number;
  bytes: number;
  /** Épisodes dont la taille est inconnue. */
  unknownSize: number;
}

export function totalsOf(selected: readonly SelectedEpisode[]): Totals {
  let bytes = 0;
  let unknownSize = 0;
  for (const item of selected) {
    if (item.bytes === null) unknownSize += 1;
    else bytes += item.bytes;
  }
  return { count: selected.length, bytes, unknownSize };
}

/** Raison pour laquelle `candidate` ne peut pas être ajouté, ou `null` (plafonds D1, N9). */
export function refusalFor(
  selected: readonly SelectedEpisode[],
  candidate: Pick<SelectedEpisode, "id" | "bytes">,
): Refusal | null {
  if (selected.some((s) => s.id === candidate.id)) return "duplicate";
  if (candidate.bytes !== null && candidate.bytes > MAX_EPISODE_BYTES) {
    return "episodeTooBig";
  }
  if (selected.length >= MAX_EPISODES) return "count";
  if (totalsOf(selected).bytes + (candidate.bytes ?? 0) > MAX_PACK_BYTES) return "total";
  return null;
}

/** Identifiants des `n` premiers épisodes (liste déjà triée par date décroissante). */
export function latestIds(episodes: readonly Pick<Episode, "id">[], n: number): string[] {
  return episodes.slice(0, Math.max(0, n)).map((e) => e.id);
}
