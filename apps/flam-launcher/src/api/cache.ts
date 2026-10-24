import { invokeCommand } from "./error";

/** Réponse de `cache_usage` (miroir de `CacheUsage` dans `src-tauri/src/cache.rs`). */
export interface CacheUsage {
  /** Dossier de cache effectif. */
  path: string;
  /** Octets occupés par les sous-dossiers créés par l'application. */
  bytes: number;
  /** Sous-dossiers que l'application compte et supprime. */
  subdirs: string[];
}

/** Réponse de `podcast_mp3_cache_size` (miroir de `Mp3CacheUsage` dans `src-tauri/src/podcast/mp3size.rs`). */
export interface Mp3CacheUsage {
  /** Octets occupés par les MP3 convertis des podcasts (déjà compris dans `CacheUsage.bytes`). */
  bytes: number;
}

/** Réponse de `cache_clear` (miroir de `CacheCleared`). */
export interface CacheCleared {
  freed_bytes: number;
}

/** `cache_usage` : espace occupé par les données de l'application dans le cache. */
export function getCacheUsage(): Promise<CacheUsage> {
  return invokeCommand<CacheUsage>("cache_usage");
}

/** `podcast_mp3_cache_size` : part du cache occupée par les MP3 de podcasts (#489, § 13.6). */
export function getMp3CacheUsage(): Promise<Mp3CacheUsage> {
  return invokeCommand<Mp3CacheUsage>("podcast_mp3_cache_size");
}

/**
 * `cache_clear` : supprime uniquement les sous-dossiers créés par l'application (jamais le
 * reste du dossier configuré). À appeler après confirmation de l'utilisateur. Refusé avec
 * `player_busy` tant qu'un player tourne.
 */
export function clearCache(): Promise<CacheCleared> {
  return invokeCommand<CacheCleared>("cache_clear");
}
