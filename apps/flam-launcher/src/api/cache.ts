import { invokeCommand } from "./error";

/** Réponse de `cache_usage` (miroir de `CacheUsage` dans `src-tauri/src/cache.rs`). */
export interface CacheUsage {
  /** Dossier de cache effectif. */
  path: string;
  /** Octets occupés par les sous-dossiers créés par l'application. */
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

/**
 * `cache_clear` : supprime uniquement les sous-dossiers créés par l'application (jamais le
 * reste du dossier configuré). À appeler après confirmation de l'utilisateur.
 */
export function clearCache(): Promise<CacheCleared> {
  return invokeCommand<CacheCleared>("cache_clear");
}
