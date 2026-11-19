import { useEffect, useState } from "react";
import { editorThumbnail, type Stamp } from "../../../api/editor";
import { Icon, type IconName } from "../../../components";

/** Vignettes déjà reçues : id + état vu + côté → URL `data:`. */
const cache = new Map<string, string>();
/** Demandes en cours au plus (décodage dans un processus enfant côté backend). */
const MAX_PARALLEL = 3;
let running = 0;
const waiting: (() => void)[] = [];

function slot(): Promise<() => void> {
  return new Promise((resolve) => {
    const start = () => {
      running += 1;
      resolve(() => {
        running -= 1;
        waiting.shift()?.();
      });
    };
    if (running < MAX_PARALLEL) start();
    else waiting.push(start);
  });
}

function key(id: string, stamp: Stamp | undefined, size: number): string {
  return `${id}|${stamp?.len ?? 0}|${stamp?.modifiedMs ?? 0}|${size}`;
}

/** Oublie les vignettes reçues (tests). */
export function clearThumbnailCache() {
  cache.clear();
}

export interface ThumbnailProps {
  /** Id de l'image ; `null` : aucune (icône seule). */
  id: string | null;
  stamp?: Stamp;
  size: number;
  /** Texte de remplacement ; vide : image décorative. */
  alt: string;
  /** Icône montrée sans image (ou avant son arrivée). */
  fallback: IconName;
  className?: string;
}

/** Vignette d'une image du dossier, demandée au backend à l'affichage. */
export function Thumbnail({ id, stamp, size, alt, fallback, className }: ThumbnailProps) {
  const wanted = id === null ? null : key(id, stamp, size);
  const [loaded, setLoaded] = useState<{ key: string; url: string | null } | null>(null);
  const url =
    wanted === null
      ? null
      : (cache.get(wanted) ?? (loaded?.key === wanted ? loaded.url : null));

  useEffect(() => {
    if (id === null || wanted === null || cache.has(wanted)) return;
    let alive = true;
    void (async () => {
      const release = await slot();
      try {
        if (!alive) return;
        const data = await editorThumbnail(id, size);
        cache.set(wanted, data);
        if (alive) setLoaded({ key: wanted, url: data });
      } catch {
        if (alive) setLoaded({ key: wanted, url: null });
      } finally {
        release();
      }
    })();
    return () => {
      alive = false;
    };
  }, [id, wanted, size]);

  return (
    <span
      className={["fl-editor-thumb", className].filter(Boolean).join(" ")}
      style={{ width: size, height: Math.round((size * 3) / 4) }}
    >
      {url ? (
        <img src={url} alt={alt} />
      ) : (
        <Icon name={fallback} size={Math.max(16, Math.round(size / 3))} />
      )}
    </span>
  );
}
