import { useCallback, useEffect, useState } from "react";
import { getCatalogThumbnail } from "../../api/catalog";
import { folderThumbnail } from "../../api/folder";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";

/** Demande la vignette d'une histoire à la commande de sa source (URL `data:`). */
function fetchThumbnail(source: Source, entry: StoryEntry): Promise<string> | null {
  const ref = entry.thumb_url;
  if (!ref) return null;
  if (ref.startsWith("data:"))
    return ref.startsWith("data:image/") ? Promise.resolve(ref) : null;
  if (source.type === "catalog") return getCatalogThumbnail(source.id, ref);
  if (source.type === "folder") return folderThumbnail(source.id, entry.entry_id);
  return null;
}

/**
 * Vignette d'une carte, chargée seulement quand la carte approche de l'écran (une source
 * peut compter des milliers d'histoires). Renvoie l'URL (ou `undefined`) et la fonction à poser
 * en `ref` sur l'élément observé. Sans `IntersectionObserver`, la vignette est demandée aussitôt.
 */
export function useThumbnail(
  source: Source,
  entry: StoryEntry,
): {
  url: string | undefined;
  observe: (node: HTMLElement | null) => (() => void) | void;
} {
  const [visible, setVisible] = useState(typeof IntersectionObserver === "undefined");
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

  const key = `${source.id}\u0000${entry.entry_id}\u0000${entry.thumb_url ?? ""}`;
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
