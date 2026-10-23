import { useCallback, useEffect, useState } from "react";
import { getCatalogThumbnail } from "../../api/catalog";
import { folderThumbnail } from "../../api/folder";
import { libraryThumbnail } from "../../api/library";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
import type { ListedEntry } from "./sourceData";

/** Demande la vignette d'une histoire à la commande de sa source (URL `data:`). */
function fetchThumbnail(source: Source, entry: StoryEntry): Promise<string> | null {
  // Histoire installée : d'abord la vignette conservée par la bibliothèque, puis (installation
  // antérieure, sans vignette conservée) celle de la source d'origine.
  if (source.type === "library") {
    return libraryThumbnail(entry.uuid).catch(() => {
      const origin = fetchFromSource(source, entry);
      return origin ?? Promise.reject(new Error("pas de vignette"));
    });
  }
  return fetchFromSource(source, entry);
}

function fetchFromSource(source: Source, entry: StoryEntry): Promise<string> | null {
  const ref = entry.thumb_url;
  if (!ref) return null;
  if (ref.startsWith("data:"))
    return ref.startsWith("data:image/") ? Promise.resolve(ref) : null;
  if (source.type === "catalog") return getCatalogThumbnail(source.id, ref);
  if (source.type === "folder") return folderThumbnail(source.id, entry.entry_id);
  // Histoire installée : la vignette se demande à la source d'origine (catalogue si l'archive
  // venait du web, sinon dossier local), qui peut avoir disparu : pas de vignette alors.
  return /^https?:\/\//i.test(entry.download_url)
    ? getCatalogThumbnail(entry.source_id, ref)
    : folderThumbnail(entry.source_id, entry.entry_id);
}

/**
 * Vignette d'une carte, chargée seulement quand la carte approche de l'écran (une source
 * peut compter des milliers d'histoires). Renvoie l'URL (ou `undefined`) et la fonction à poser
 * en `ref` sur l'élément observé. Sans `IntersectionObserver`, la vignette est demandée aussitôt.
 * Avec `eager` (fiche ouverte, donc déjà à l'écran et sans élément à observer), elle l'est aussi.
 */
export function useThumbnail(
  source: Source,
  entry: ListedEntry,
  { eager = false }: { eager?: boolean } = {},
): {
  url: string | undefined;
  observe: (node: HTMLElement | null) => (() => void) | void;
} {
  const [observed, setVisible] = useState(typeof IntersectionObserver === "undefined");
  const visible = eager || observed;
  const [loaded, setLoaded] = useState<{ key: string; url: string }>();

  const observe = useCallback((node: HTMLElement | null) => {
    if (!node || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (records) => {
        if (records.some((record) => record.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  // « Mes histoires » : la vignette conservée change à chaque réinstallation (nom unique par
  // installation) ; elle entre dans la clé pour que l'image se rafraîchisse sans remontage.
  const kept = source.type === "library" ? (entry.library_thumb ?? "") : "";
  const key = `${source.id}\u0000${entry.entry_id}\u0000${entry.thumb_url ?? ""}\u0000${kept}`;
  useEffect(() => {
    if (!visible) return;
    const request = fetchThumbnail(source, entry);
    if (!request) return;
    let active = true;
    request
      .then((url) => {
        if (active && url.startsWith("data:image/")) setLoaded({ key, url });
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [visible, key, source, entry]);

  return { url: loaded?.key === key ? loaded.url : undefined, observe };
}
