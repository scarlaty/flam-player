import type { Episode } from "../../api/podcast";

/** Plafonds fixes par pack (D1). 1 Mo = 10⁶ octets ; 1 Gio = 2³⁰ octets. */
export const MAX_EPISODES = 50;
export const MAX_EPISODE_BYTES = 200_000_000;
export const MAX_PACK_BYTES = 1_073_741_824;

/** Débit supposé quand la taille n'est pas annoncée : 192 kbit/s, le cas le plus coûteux (D5, D6). */
const FALLBACK_BYTES_PER_SECOND = 24_000;

/** Raccourci « N derniers épisodes » : 10 par défaut ; la sélection reste libre (#220, spec § 9). */
export const LATEST_DEFAULT_COUNT = 10;

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

/**
 * Clé d'un épisode dans la sélection : l'identifiant d'épisode n'est unique qu'au sein de son
 * flux (source + `guid`), deux flux peuvent donc en partager un sans que ce soit le même épisode.
 */
export function episodeKey(sourceId: string, id: string): string {
  return `${sourceId}\u0000${id}`;
}

/** Épisode retenu pour le pack (instantané : reste valable si le flux est rafraîchi). */
export interface SelectedEpisode {
  id: string;
  sourceId: string;
  sourceName: string;
  title: string;
  /**
   * Taille estimée en octets ; `null` : inconnue, comptée pour 0 dans le total. Le plafond de
   * 1 Gio peut donc être dépassé par des épisodes sans taille ni durée : l'écran le signale
   * (« non comptés ») et la commande « créer un pack » (#217, spec § 5) devra revalider entièrement la composition
   * dans le backend, plafond de 1 Gio compris sur les tailles réelles.
   */
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
  candidate: Pick<SelectedEpisode, "id" | "sourceId" | "bytes">,
): Refusal | null {
  const key = episodeKey(candidate.sourceId, candidate.id);
  if (selected.some((s) => episodeKey(s.sourceId, s.id) === key)) return "duplicate";
  if (candidate.bytes !== null && candidate.bytes > MAX_EPISODE_BYTES) {
    return "episodeTooBig";
  }
  if (selected.length >= MAX_EPISODES) return "count";
  if (totalsOf(selected).bytes + (candidate.bytes ?? 0) > MAX_PACK_BYTES) return "total";
  return null;
}

/** Issue du raccourci « N derniers » : épisodes cochés et cause de chaque refus. */
export interface ShortcutOutcome {
  accepted: SelectedEpisode[];
  /** Déjà cochés avant le raccourci. */
  already: number;
  /** Explicites, non cochés d'office (D8). */
  explicit: number;
  /** Refusés par les plafonds (nombre, taille de l'épisode ou du pack : D1, N9). */
  capped: number;
}

/**
 * Épisodes que le raccourci « N derniers » peut cocher : parcourt `candidates` dans l'ordre et
 * garde ceux que les plafonds acceptent (D1, N9), en tenant compte de ceux déjà retenus dans la
 * foulée. Les épisodes déjà cochés sont comptés à part, puis les explicites (non cochés
 * d'office, D8), puis ceux que les plafonds refusent.
 */
export function shortcutOutcome(
  selected: readonly SelectedEpisode[],
  candidates: readonly SelectedEpisode[],
): ShortcutOutcome {
  const outcome: ShortcutOutcome = { accepted: [], already: 0, explicit: 0, capped: 0 };
  for (const candidate of candidates) {
    const refusal = refusalFor([...selected, ...outcome.accepted], candidate);
    if (refusal === "duplicate") outcome.already += 1;
    else if (candidate.explicit) outcome.explicit += 1;
    else if (refusal !== null) outcome.capped += 1;
    else outcome.accepted.push(candidate);
  }
  return outcome;
}
