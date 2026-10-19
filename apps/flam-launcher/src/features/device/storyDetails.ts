import { useEffect, useRef, useState, type RefObject } from "react";
import {
  STORY_TITLES_BATCH,
  cancelDeviceStoryDetails,
  getDeviceStoryThumbnail,
  getDeviceStoryTitles,
  type DeviceStoryThumbnail,
} from "../../api/device";

/** Délai avant de redemander ce qu'une opération en cours sur la Flam a interrompu. */
export const DETAILS_RETRY_MS = 2000;
/** Vignettes lues en même temps sur la Flam (clé USB lente : peu à la fois). */
export const THUMB_CONCURRENCY = 2;

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Titres déchiffrés des histoires `uuids` de la Flam de `root`, chargés par lots en tâche de
 * fond après l'affichage de la liste. Une histoire absente de la table n'a pas encore de
 * réponse ; `null` : pas de titre lisible (repli sur la bibliothèque, puis l'UUID court).
 *
 * `paused` (envoi ou retrait en cours) suspend les demandes. Le chargement s'arrête au
 * changement de Flam ou au démontage, et le lot en cours est interrompu côté Rust. Un échec
 * n'est pas affiché : les titres se replient.
 */
export function useDeviceStoryTitles(
  root: string | null,
  uuids: string[],
  paused: boolean,
): ReadonlyMap<string, string | null> {
  const [titles, setTitles] = useState<{
    root: string | null;
    map: ReadonlyMap<string, string | null>;
  }>({ root: null, map: EMPTY_TITLES });
  // Réponses déjà reçues pour la Flam en cours (lu et écrit seulement dans l'effet).
  const known = useRef<{ root: string | null; map: Map<string, string | null> }>({
    root: null,
    map: new Map(),
  });
  const uuidsKey = uuids.join(" ");

  useEffect(() => {
    if (!root || paused) return;
    if (known.current.root !== root) known.current = { root, map: new Map() };
    const map = known.current.map;
    let pending = uuidsKey ? uuidsKey.split(" ").filter((u) => !map.has(u)) : [];
    if (pending.length === 0) return;
    let active = true;
    let inFlight = false;
    void (async () => {
      while (active && pending.length > 0) {
        const batch = pending.slice(0, STORY_TITLES_BATCH);
        inFlight = true;
        let reply;
        try {
          reply = await getDeviceStoryTitles(root, batch);
        } catch {
          return;
        } finally {
          inFlight = false;
        }
        if (!active) return;
        for (const { uuid, title } of reply.titles) map.set(uuid, title);
        // Lot complet : une histoire omise (UUID invalide) n'aura pas de titre.
        if (!reply.interrupted)
          for (const uuid of batch) if (!map.has(uuid)) map.set(uuid, null);
        setTitles({ root, map: new Map(map) });
        pending = pending.filter((u) => !map.has(u));
        if (reply.interrupted) await delay(DETAILS_RETRY_MS);
      }
    })();
    return () => {
      active = false;
      if (inFlight) void cancelDeviceStoryDetails().catch(() => undefined);
    };
  }, [root, uuidsKey, paused]);

  return titles.root === root ? titles.map : EMPTY_TITLES;
}

const EMPTY_TITLES: ReadonlyMap<string, string | null> = new Map();

type Fetcher = (root: string, uuid: string) => Promise<DeviceStoryThumbnail>;

/**
 * File des vignettes d'une Flam : au plus `THUMB_CONCURRENCY` lectures à la fois, résultats
 * gardés en mémoire, nouvel essai après `DETAILS_RETRY_MS` quand la Flam est occupée.
 * `dispose` abandonne les demandes en attente (changement de Flam, démontage).
 */
export class ThumbnailLoader {
  private readonly done = new Map<string, string | null>();
  private readonly waiting = new Map<string, Array<(url: string | null) => void>>();
  private readonly queue: string[] = [];
  private running = 0;
  private paused = false;
  private disposed = false;

  constructor(
    private readonly root: string,
    private readonly fetcher: Fetcher = getDeviceStoryThumbnail,
  ) {}

  /** Vignette déjà connue (`undefined` : pas encore lue). */
  peek(uuid: string): string | null | undefined {
    return this.done.get(uuid);
  }

  /** Demande la vignette `uuid` ; `null` si absente, invalide ou abandonnée. */
  request(uuid: string): Promise<string | null> {
    const known = this.done.get(uuid);
    if (known !== undefined) return Promise.resolve(known);
    return new Promise((resolve) => {
      const waiters = this.waiting.get(uuid);
      if (waiters) {
        waiters.push(resolve);
        return;
      }
      this.waiting.set(uuid, [resolve]);
      this.queue.push(uuid);
      this.pump();
    });
  }

  setPaused(paused: boolean): void {
    this.paused = paused;
    this.pump();
  }

  dispose(): void {
    this.disposed = true;
    this.queue.length = 0;
    for (const waiters of this.waiting.values())
      for (const resolve of waiters) resolve(null);
    this.waiting.clear();
  }

  private finish(uuid: string, url: string | null): void {
    this.done.set(uuid, url);
    for (const resolve of this.waiting.get(uuid) ?? []) resolve(url);
    this.waiting.delete(uuid);
  }

  private pump(): void {
    while (!this.disposed && !this.paused && this.running < THUMB_CONCURRENCY) {
      const uuid = this.queue.shift();
      if (uuid === undefined) return;
      this.running += 1;
      void this.load(uuid);
    }
  }

  private async load(uuid: string): Promise<void> {
    let retry = false;
    try {
      const reply = await this.fetcher(this.root, uuid);
      if (this.disposed) return;
      if (reply.busy) {
        retry = true;
      } else {
        const url = reply.thumb?.startsWith("data:image/") ? reply.thumb : null;
        this.finish(uuid, url);
      }
    } catch {
      if (!this.disposed) this.finish(uuid, null);
    } finally {
      this.running -= 1;
    }
    if (retry) {
      await delay(DETAILS_RETRY_MS);
      if (this.disposed) return;
      this.queue.push(uuid);
    }
    this.pump();
  }
}

/**
 * Vignette de l'histoire `uuid`, demandée quand sa ligne `ref` approche de la zone visible
 * (aussitôt si `IntersectionObserver` manque).
 */
export function useDeviceThumbnail(
  loader: ThumbnailLoader | null,
  uuid: string,
  ref: RefObject<Element | null>,
): string | undefined {
  const [thumb, setThumb] = useState<{ uuid: string; url: string | null }>();
  const known = loader?.peek(uuid);
  useEffect(() => {
    if (!loader || loader.peek(uuid) !== undefined) return;
    let active = true;
    const load = () => {
      void loader.request(uuid).then((url) => {
        if (active) setThumb({ uuid, url });
      });
    };
    const element = ref.current;
    if (typeof IntersectionObserver === "undefined" || !element) {
      load();
      return () => {
        active = false;
      };
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          load();
        }
      },
      { rootMargin: "300px" },
    );
    observer.observe(element);
    return () => {
      active = false;
      observer.disconnect();
    };
  }, [loader, uuid, ref]);
  if (known !== undefined) return known ?? undefined;
  return thumb?.uuid === uuid ? (thumb.url ?? undefined) : undefined;
}
