import { useSyncExternalStore } from "react";

/** Écrans de l'application, désignés par le hash de l'URL (`#/sources`…). */
export type Route =
  "home" | "sources" | "settings" | "library" | "device" | "podcasts" | "pack" | "demo";

/** Adresse de la page de démonstration des composants (développement uniquement). */
export const DEMO_HASH = "#/demo";

/** Éditeur de pack de podcasts (#363), sous l'écran Podcasts. */
export const PACK_HASH = "#/podcasts/pack";

export const ROUTE_HASHES = {
  home: "#/",
  sources: "#/sources",
  settings: "#/settings",
  device: "#/device",
  podcasts: "#/podcasts",
} as const;

/** Préfixe de la bibliothèque : `#/library` (première source) ou `#/library/<id de source>`. */
const LIBRARY_PREFIX = "#/library";

/** Hash de la vue d'une source (sans identifiant : la première source). */
export function libraryHash(sourceId?: string): string {
  return sourceId === undefined
    ? LIBRARY_PREFIX
    : `${LIBRARY_PREFIX}/${encodeURIComponent(sourceId)}`;
}

/** Identifiant de source désigné par un hash `#/library/<id>` ; `null` sinon. */
export function sourceIdFromHash(hash: string): string | null {
  if (!hash.startsWith(`${LIBRARY_PREFIX}/`)) return null;
  const raw = hash.slice(LIBRARY_PREFIX.length + 1);
  if (raw === "") return null;
  try {
    return decodeURIComponent(raw);
  } catch {
    return null;
  }
}

/** Écran correspondant à un hash ; tout hash inconnu mène à l'accueil. */
export function routeFromHash(hash: string): Route {
  if (hash === LIBRARY_PREFIX || hash.startsWith(`${LIBRARY_PREFIX}/`)) return "library";
  if (hash === ROUTE_HASHES.sources) return "sources";
  if (hash === ROUTE_HASHES.settings) return "settings";
  if (hash === ROUTE_HASHES.device) return "device";
  if (hash === ROUTE_HASHES.podcasts) return "podcasts";
  if (hash === PACK_HASH) return "pack";
  if (hash === DEMO_HASH) return "demo";
  return "home";
}

function subscribeHash(callback: () => void) {
  window.addEventListener("hashchange", callback);
  return () => window.removeEventListener("hashchange", callback);
}

/** Écran courant, mis à jour à chaque changement de hash. */
export function useRoute(): Route {
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);
  return routeFromHash(hash);
}

/** Source désignée par le hash courant (`null` : la première). */
export function useSourceId(): string | null {
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);
  return sourceIdFromHash(hash);
}
