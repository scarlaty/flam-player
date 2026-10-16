import { useSyncExternalStore } from "react";

/** Écrans de l'application, désignés par le hash de l'URL (`#/sources`…). */
export type Route = "home" | "sources" | "settings" | "demo";

/** Adresse de la page de démonstration des composants (développement uniquement). */
export const DEMO_HASH = "#/demo";

export const ROUTE_HASHES = {
  home: "#/",
  sources: "#/sources",
  settings: "#/settings",
} as const;

/** Écran correspondant à un hash ; tout hash inconnu mène à l'accueil. */
export function routeFromHash(hash: string): Route {
  if (hash === ROUTE_HASHES.sources) return "sources";
  if (hash === ROUTE_HASHES.settings) return "settings";
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
