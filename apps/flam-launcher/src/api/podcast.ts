import type { CatalogStatus } from "./catalog";
import { invokeCommand } from "./error";
import type { Source } from "./sources";

/** Épisode d'un flux (miroir de `EpisodeView` dans `src-tauri/src/podcast/source.rs`). */
export interface Episode {
  /** Identifiant opaque et stable. */
  id: string;
  title: string;
  description: string | null;
  /** ISO 8601 UTC ; `null` si inconnue. */
  pub_date: string | null;
  /** Durée en secondes, si connue. */
  duration: number | null;
  author: string | null;
  enclosure_type: string;
  /** Taille annoncée de l'enclosure, en octets. */
  enclosure_length: number | null;
  has_image: boolean;
  /**
   * Contenu explicite (D8). Absent tant que le backend ne l'expose pas dans `EpisodeView` :
   * un épisode sans valeur est traité comme de statut inconnu (ni signalé, ni décoché de plus).
   */
  explicit?: boolean | null;
}

/** Contenu d'un flux, épisodes par date décroissante (miroir de `FeedView`). */
export interface Feed {
  title: string;
  description: string | null;
  copyright: string | null;
  has_image: boolean;
  episodes: Episode[];
  warnings: string[];
}

/** Flux d'une source et fraîcheur (miroir de `PodcastView`) ; `feed` nul : jamais obtenu. */
export interface PodcastView {
  source_id: string;
  feed: Feed | null;
  status: CatalogStatus;
}

/** Réponse de `podcast_add`. */
export interface PodcastAdded {
  source: Source;
  view: PodcastView;
}

/** `podcast_add` : valide l'adresse, télécharge le flux puis enregistre la source. */
export function addPodcast(url: string, name?: string): Promise<PodcastAdded> {
  return invokeCommand<PodcastAdded>("podcast_add", { url, name: name ?? null });
}

/** `podcast_feed` : flux en cache, sans réseau. */
export function getPodcastFeed(sourceId: string): Promise<PodcastView> {
  return invokeCommand<PodcastView>("podcast_feed", { sourceId });
}

/** `podcast_refresh` : rafraîchit le flux (cache et `status.offline` sans réseau). */
export function refreshPodcast(sourceId: string): Promise<PodcastView> {
  return invokeCommand<PodcastView>("podcast_refresh", { sourceId });
}
