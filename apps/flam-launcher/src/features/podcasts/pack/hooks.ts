import { useCallback, useEffect, useRef, useState } from "react";
import { toAppError, type AppError } from "../../../api/error";
import {
  savePackDraft,
  validatePack,
  type PackComposition,
  type ValidationReport,
} from "../../../api/pack";
import { getPodcastFeed, type Episode } from "../../../api/podcast";
import { onWindowCloseRequested } from "../../../api/window";

export type SaveStatus = "idle" | "saving" | "saved" | "error";

/** Nouvelles tentatives d'une sauvegarde dont la révision a été devancée (une seule suffit). */
const MAX_STALE_RETRIES = 3;

/** Temps laissé à la sauvegarde de fermeture avant de laisser la fenêtre se fermer. */
export const CLOSE_FLUSH_MS = 1000;

/** Attente maximale, à l'ouverture de l'éditeur, de la sauvegarde d'un éditeur fermé. */
const SETTLE_SAVE_MS = 2000;

/** Sauvegarde en cours (au plus une : le brouillon est unique), toutes instances du hook. */
let pendingSave: Promise<void> | null = null;

/**
 * Attend la sauvegarde lancée par un éditeur qui vient de se fermer : à rouvrir l'éditeur
 * aussitôt, la lecture du brouillon doit voir cette sauvegarde, pas la précédente.
 */
export async function settlePendingSave(): Promise<void> {
  let giveUp: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<void>((resolve) => {
    giveUp = setTimeout(resolve, SETTLE_SAVE_MS);
  });
  try {
    // Borné : une sauvegarde qui ne répond pas ne doit pas empêcher d'ouvrir l'éditeur.
    await Promise.race([
      timeout,
      (async () => {
        while (pendingSave !== null) await pendingSave.catch(() => {});
      })(),
    ]);
  } finally {
    clearTimeout(giveUp);
  }
}

/**
 * Sauvegarde automatique du brouillon (N3 b) : après `delayMs` sans modification, une seule
 * sauvegarde à la fois, avec la révision renvoyée par le backend (`init` à la lecture, puis
 * celle de chaque réponse). Une sauvegarde en attente est lancée au démontage et à la
 * demande de fermeture de la fenêtre (attendue au plus `CLOSE_FLUSH_MS`).
 */
export function useDraftAutosave(delayMs: number) {
  const [status, setStatus] = useState<SaveStatus>("idle");
  const revision = useRef(0);
  const latest = useRef<PackComposition | null>(null);
  const dirty = useRef(false);
  const running = useRef<Promise<void> | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(false);

  const save = useCallback(async () => {
    let stale = 0;
    while (dirty.current && latest.current) {
      dirty.current = false;
      if (mounted.current) setStatus("saving");
      try {
        const outcome = await savePackDraft(latest.current, revision.current + 1);
        revision.current = outcome.revision;
        if (!outcome.saved) {
          // Éditeur fermé : le brouillon a été remplacé entre-temps (composition ouverte,
          // éditeur rouvert, #484). Réessayer au-dessus écraserait ce remplacement.
          if (!mounted.current) return;
          stale += 1;
          if (stale >= MAX_STALE_RETRIES) throw new Error("révision devancée");
          dirty.current = true;
        } else if (mounted.current && !dirty.current) {
          setStatus("saved");
        }
      } catch {
        dirty.current = true;
        if (mounted.current) setStatus("error");
        return;
      }
    }
  }, []);

  /** Lance la sauvegarde (ou rejoint celle en cours, qui reprend tant que c'est modifié). */
  const flush = useCallback((): Promise<void> => {
    if (running.current) return running.current;
    const run = save().finally(() => {
      running.current = null;
      if (pendingSave === run) pendingSave = null;
    });
    running.current = run;
    pendingSave = run;
    return run;
  }, [save]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      clearTimeout(timer.current);
      if (dirty.current) void flush();
    };
  }, [flush]);

  // Fermeture de la fenêtre : le débounce est vidé avant qu'elle ne se ferme (800 ms de
  // modifications seraient perdues sinon). L'attente est bornée : un disque lent ne bloque pas.
  useEffect(() => {
    let off: (() => void) | undefined;
    let disposed = false;
    void onWindowCloseRequested(async () => {
      clearTimeout(timer.current);
      if (!dirty.current && running.current === null) return;
      let giveUp: ReturnType<typeof setTimeout> | undefined;
      await Promise.race([
        flush(),
        new Promise<void>((resolve) => {
          giveUp = setTimeout(resolve, CLOSE_FLUSH_MS);
        }),
      ]);
      clearTimeout(giveUp);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else off = unlisten;
    });
    return () => {
      disposed = true;
      off?.();
    };
  }, [flush]);

  /** Révision enregistrée lue avec le brouillon. */
  const init = useCallback((stored: number) => {
    revision.current = stored;
  }, []);

  /** Demande l'enregistrement de `composition` (regroupé avec les modifications proches). */
  const schedule = useCallback(
    (composition: PackComposition) => {
      latest.current = composition;
      dirty.current = true;
      clearTimeout(timer.current);
      timer.current = setTimeout(() => void flush(), delayMs);
    },
    [delayMs, flush],
  );

  /** Nouvelle tentative après un échec. */
  const retry = useCallback(() => {
    clearTimeout(timer.current);
    void flush();
  }, [flush]);

  /**
   * Enregistre sans attendre ce qui est en attente puis renvoie la révision enregistrée et la composition correspondante (celle
   * que `pack_create` compare pour effacer le brouillon). Si l'enregistrement échoue, la
   * dernière révision connue est renvoyée : le brouillon sera alors conservé.
   */
  const settle = useCallback(async () => {
    clearTimeout(timer.current);
    await flush();
    // `composition` : la plus récente demandée, celle que `revision` décrit.
    return { revision: revision.current, composition: latest.current };
  }, [flush]);

  return { status, init, schedule, retry, settle };
}

interface ValidationResult {
  for: PackComposition;
  report: ValidationReport | null;
  error: AppError | null;
}

/**
 * Validation du pack par le backend (`pack_validate`), espacée : un appel `delayMs` après la
 * dernière modification, jamais à chaque frappe. Le dernier rapport reste affiché pendant
 * le calcul (`pending`).
 */
export function useDebouncedValidation(
  composition: PackComposition | null,
  delayMs: number,
) {
  const [result, setResult] = useState<ValidationResult | null>(null);
  const seeded = useRef<PackComposition | null>(null);

  /** Rapport déjà connu de `known` (lu avec le brouillon) : pas de nouvel appel. */
  const seed = useCallback((known: PackComposition, report: ValidationReport) => {
    seeded.current = known;
    setResult({ for: known, report, error: null });
  }, []);

  useEffect(() => {
    if (composition === null || composition === seeded.current) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      validatePack(composition)
        .then((report) => {
          if (!cancelled) setResult({ for: composition, report, error: null });
        })
        .catch((caught: unknown) => {
          if (!cancelled)
            setResult({ for: composition, report: null, error: toAppError(caught) });
        });
    }, delayMs);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [composition, delayMs]);

  const fresh = result !== null && result.for === composition;
  return {
    report: result?.report ?? null,
    /** Composition que `report` décrit (ses chemins valent pour celle-ci). */
    reportFor: result?.for ?? null,
    error: fresh ? result.error : null,
    pending: !fresh,
    seed,
  };
}

/** Ce que le flux en cache apprend sur une source (titres, images, défauts D16). */
export interface FeedInfo {
  sourceId: string;
  title: string;
  copyright: string | null;
  hasImage: boolean;
  episodes: ReadonlyMap<string, Episode>;
}

/** Flux en cache (sans réseau) des sources demandées ; `null` : flux inconnu ou illisible. */
export function useFeedInfo(sourceIds: readonly string[]) {
  const [infos, setInfos] = useState<ReadonlyMap<string, FeedInfo | null>>(new Map());
  const requested = useRef(new Set<string>());
  const mounted = useRef(false);
  const key = [...sourceIds].sort().join("\u0000");

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    for (const id of key === "" ? [] : key.split("\u0000")) {
      if (requested.current.has(id)) continue;
      requested.current.add(id);
      const store = (info: FeedInfo | null) => {
        if (mounted.current) setInfos((prev) => new Map(prev).set(id, info));
      };
      getPodcastFeed(id)
        .then((view) =>
          store(
            view.feed === null
              ? null
              : {
                  sourceId: id,
                  title: view.feed.title,
                  copyright: view.feed.copyright,
                  hasImage: view.feed.has_image,
                  episodes: new Map(view.feed.episodes.map((e) => [e.id, e])),
                },
          ),
        )
        .catch(() => store(null));
    }
  }, [key]);

  return infos;
}
