import { useEffect, useRef, useState, type RefObject } from "react";
import {
  STORY_TITLES_BATCH,
  cancelDeviceStoryDetails,
  getDeviceStoryThumbnail,
  getDeviceStoryTitles,
  getOfficialStoryThumbnail,
  getOfficialStoryTitles,
  type DeviceStoryThumbnail,
} from "../../api/device";
import type { QueueItem } from "./DeviceContext";

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
 *
 * `lunii` : histoires au format Lunii/STUdio (`info` en clair, #522). Leur titre est lu sans
 * clé, donc retenu même quand la Flam n'a pas de clé de titre.
 *
 * `sent` (voir `sentStamps`) : envois terminés, UUID et numéro de relecture. Le titre d'un UUID
 * dont un envoi vient de se terminer (« Remplacer sur la Flam » garde l'UUID) est oublié, donc
 * relu, une fois par envoi.
 *
 * `keysProblem` : les clés de titre d'une Flam 2.x n'ont pas pu être lues (#317) ; les titres
 * se replient alors sur la bibliothèque ou le catalogue.
 */
export function useDeviceStoryTitles(
  root: string | null,
  uuids: string[],
  paused: boolean,
  sent: ReadonlyMap<string, number> = EMPTY_SENT,
): DeviceStoryTitlesState {
  const [titles, setTitles] = useState<{
    root: string | null;
    map: ReadonlyMap<string, string | null>;
    lunii: ReadonlySet<string>;
    keysProblem: string | null;
  }>({ root: null, map: EMPTY_TITLES, lunii: EMPTY_LUNII, keysProblem: null });
  // Réponses déjà reçues pour la Flam en cours (lu et écrit seulement dans l'effet).
  const known = useRef<{
    root: string | null;
    map: Map<string, string | null>;
    lunii: Set<string>;
    /** Dernier envoi terminé pris en compte, par UUID. */
    sent: Map<string, number>;
    keysProblem: string | null;
  }>({
    root: null,
    map: new Map(),
    lunii: new Set(),
    sent: new Map(),
    keysProblem: null,
  });
  const uuidsKey = uuids.join(" ");
  const sentKey = [...sent].map(([uuid, gen]) => `${uuid}:${gen}`).join(" ");

  useEffect(() => {
    if (!root) return;
    if (known.current.root !== root)
      known.current = {
        root,
        map: new Map(),
        lunii: new Set(),
        sent: new Map(),
        keysProblem: null,
      };
    const state = known.current;
    const { map, lunii } = state;
    const publish = () =>
      setTitles({
        root,
        map: new Map(map),
        lunii: new Set(lunii),
        keysProblem: state.keysProblem,
      });
    // Histoire retirée de la liste : sa réponse (titre, format) est oubliée. Un même UUID
    // réinstallé dans un autre format (Lunii puis Flam, ou l'inverse) est donc relu, sans
    // badge ni titre périmés.
    const current = new Set(uuidsKey ? uuidsKey.split(" ") : []);
    let pruned = false;
    for (const uuid of [...map.keys()])
      if (!current.has(uuid)) {
        map.delete(uuid);
        lunii.delete(uuid);
        pruned = true;
      }
    // Envoi terminé pour cet UUID : le titre lu avant l'envoi n'est plus fiable.
    for (const entry of sentKey ? sentKey.split(" ") : []) {
      const cut = entry.lastIndexOf(":");
      const uuid = entry.slice(0, cut);
      const gen = Number(entry.slice(cut + 1));
      if (state.sent.get(uuid) === gen) continue;
      state.sent.set(uuid, gen);
      if (map.delete(uuid)) pruned = true;
      lunii.delete(uuid);
    }
    if (pruned) publish();
    if (paused) return;
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
        // Échec de lecture des clés 2.x : signalé tant qu'il dure ; une réponse interrompue
        // sans cause (opération en cours) ne dit rien de plus.
        if (reply.keys_problem) state.keysProblem = reply.keys_problem;
        else if (!reply.interrupted) state.keysProblem = null;
        // Histoires Lunii : titre en clair, définitif même sans clé de titre.
        for (const { uuid, title, format } of reply.titles)
          if (format === "lunii") {
            map.set(uuid, title);
            lunii.add(uuid);
          }
        // Sans clé de titre, les autres réponses ne sont pas définitives (clé carrier
        // conservée ou consentement donné plus tard) : rien n'est retenu, l'effet suivant
        // (fin d'un envoi, nouvelle liste) redemandera. L'affichage se replie d'ici là.
        if (!reply.keys && !reply.interrupted) {
          publish();
          // Lots suivants : les histoires Lunii y ont un titre en clair même sans clé.
          pending = pending.slice(batch.length);
          continue;
        }
        // Lot interrompu sans clé : un titre `null` n'est pas définitif (il ne vient que de
        // l'absence de clé), seuls les titres lus sont retenus.
        for (const { uuid, title } of reply.titles)
          if (reply.keys || title !== null) map.set(uuid, title);
        // Lot complet : une histoire omise (UUID invalide) n'aura pas de titre.
        if (!reply.interrupted)
          for (const uuid of batch) if (!map.has(uuid)) map.set(uuid, null);
        publish();
        pending = pending.filter((u) => !map.has(u));
        if (reply.interrupted) await delay(DETAILS_RETRY_MS);
      }
    })();
    return () => {
      active = false;
      if (inFlight) void cancelDeviceStoryDetails().catch(() => undefined);
    };
  }, [root, uuidsKey, paused, sentKey]);

  return titles.root === root
    ? { titles: titles.map, lunii: titles.lunii, keysProblem: titles.keysProblem }
    : EMPTY_STATE;
}

/** Titres lus sur la Flam et histoires au format Lunii (voir `useDeviceStoryTitles`). */
export interface DeviceStoryTitlesState {
  titles: ReadonlyMap<string, string | null>;
  lunii: ReadonlySet<string>;
  /** Clés de titre 2.x illisibles (message sans donnée de l'appareil), sinon `null`. */
  keysProblem: string | null;
}

const EMPTY_TITLES: ReadonlyMap<string, string | null> = new Map();
const EMPTY_LUNII: ReadonlySet<string> = new Set();
const EMPTY_SENT: ReadonlyMap<string, number> = new Map();
const EMPTY_STATE: DeviceStoryTitlesState = {
  titles: EMPTY_TITLES,
  lunii: EMPTY_LUNII,
  keysProblem: null,
};

/**
 * Envois terminés de la file : UUID envoyé et UUID remplacé, avec le numéro de relecture de la
 * Flam qui suit l'envoi (différent à chaque envoi réussi). Sert à oublier titre et vignette
 * lus avant l'envoi.
 */
export function sentStamps(queue: readonly QueueItem[]): ReadonlyMap<string, number> {
  const stamps = new Map<string, number>();
  for (const item of queue) {
    if (item.state !== "done" || item.reloadGen === undefined) continue;
    for (const uuid of [item.uuid, item.replace])
      if (uuid) stamps.set(uuid, Math.max(stamps.get(uuid) ?? 0, item.reloadGen));
  }
  return stamps;
}

/** Titre du catalogue officiel d'une histoire dont le titre n'a pas pu être déchiffré. */
export interface OfficialInfo {
  title: string;
  /** Le catalogue a une image : repli si la Flam n'a pas de vignette. */
  hasImage: boolean;
}

const EMPTY_OFFICIAL: ReadonlyMap<string, OfficialInfo> = new Map();

/**
 * Titres du catalogue officiel en ligne, demandés **seulement** pour les histoires dont le titre
 * a été définitivement refusé par le déchiffrement (`null` dans `deviceTitles`) : si tous les
 * titres sont connus, rien n'est demandé et le réseau n'est pas utilisé. Chaque histoire n'est
 * demandée qu'une fois ; les histoires `lunii` (tierces, absentes du catalogue) ne sont jamais
 * demandées ; l'affichage n'attend jamais cette réponse, et un échec n'est pas
 * affiché (repli sur la bibliothèque, puis l'UUID court).
 */
export function useOfficialStoryTitles(
  deviceTitles: ReadonlyMap<string, string | null>,
  lunii: ReadonlySet<string> = EMPTY_LUNII,
): ReadonlyMap<string, OfficialInfo> {
  const [official, setOfficial] =
    useState<ReadonlyMap<string, OfficialInfo>>(EMPTY_OFFICIAL);
  const asked = useRef(new Set<string>());
  const unresolved = [...deviceTitles]
    .filter(([uuid, title]) => title === null && !lunii.has(uuid))
    .map(([uuid]) => uuid)
    .sort()
    .join(" ");

  useEffect(() => {
    const known = asked.current;
    const fresh = unresolved.split(" ").filter((u) => u && !known.has(u));
    if (fresh.length === 0) return;
    for (const uuid of fresh) known.add(uuid);
    let active = true;
    let settled = false;
    void getOfficialStoryTitles(fresh)
      .then((reply) => {
        settled = true;
        if (!active || reply.titles.length === 0) return;
        setOfficial((previous) => {
          const next = new Map(previous);
          for (const { uuid, title, has_image } of reply.titles)
            next.set(uuid, { title, hasImage: has_image });
          return next;
        });
      })
      .catch(() => {
        settled = true;
      });
    return () => {
      active = false;
      // Démontage (ou nouvelle liste) avant la réponse : ces histoires seront redemandées.
      if (!settled) for (const uuid of fresh) known.delete(uuid);
    };
  }, [unresolved]);

  return official;
}

/**
 * Images du catalogue déjà demandées (une seule demande par histoire et par session). Un échec
 * (réseau coupé, image absente) est mémorisé comme un résultat : l'image n'est pas redemandée
 * avant le prochain lancement du launcher, pour ne pas solliciter le serveur en boucle.
 */
const officialThumbs = new Map<string, Promise<string | null>>();

/** Vide la mémoire des images du catalogue (tests). */
export function resetOfficialThumbnails(): void {
  officialThumbs.clear();
}

/**
 * Image du catalogue officiel de l'histoire `uuid`, demandée seulement si `enabled` (la Flam n'a
 * pas de vignette et le catalogue a une image). `undefined` tant qu'elle n'est pas là.
 */
export function useOfficialThumbnail(uuid: string, enabled: boolean): string | undefined {
  const [thumb, setThumb] = useState<{ uuid: string; url: string | null }>();
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let request = officialThumbs.get(uuid);
    if (!request) {
      request = getOfficialStoryThumbnail(uuid).then(
        (url) => (url?.startsWith("data:image/") ? url : null),
        () => null,
      );
      officialThumbs.set(uuid, request);
    }
    void request.then((url) => {
      if (active) setThumb({ uuid, url });
    });
    return () => {
      active = false;
    };
  }, [uuid, enabled]);
  return enabled && thumb?.uuid === uuid ? (thumb.url ?? undefined) : undefined;
}

type Fetcher = (root: string, uuid: string) => Promise<DeviceStoryThumbnail>;

/**
 * Pile des vignettes d'une Flam (dernière demandée, première lue) : au plus `THUMB_CONCURRENCY` lectures à la fois, résultats
 * gardés en mémoire, nouvel essai après `DETAILS_RETRY_MS` quand la Flam est occupée.
 * `dispose` abandonne les demandes en attente (démontage) ; le chargeur reste utilisable
 * ensuite (double montage de `StrictMode`).
 */
export class ThumbnailLoader {
  private readonly done = new Map<string, string | null>();
  /** Dernier envoi terminé pris en compte, par UUID (voir `forgetSent`). */
  private readonly sent = new Map<string, number>();
  private readonly waiting = new Map<string, Array<(url: string | null) => void>>();
  private readonly queue: string[] = [];
  private running = 0;
  private paused = false;
  /** Incrémenté par `dispose` : les nouveaux essais d'avant sont abandonnés. */
  private epoch = 0;

  constructor(
    private readonly root: string,
    private readonly fetcher: Fetcher = getDeviceStoryThumbnail,
  ) {}

  /** Vignette déjà connue (`undefined` : pas encore lue). */
  peek(uuid: string): string | null | undefined {
    return this.done.get(uuid);
  }

  /**
   * Oublie les vignettes des histoires dont un envoi vient de se terminer (`sentStamps`) : elles
   * seront relues. Un même envoi n'est pris en compte qu'une fois.
   */
  forgetSent(stamps: ReadonlyMap<string, number>): void {
    for (const [uuid, gen] of stamps) {
      if (this.sent.get(uuid) === gen) continue;
      this.sent.set(uuid, gen);
      this.done.delete(uuid);
    }
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
    this.epoch += 1;
    this.queue.length = 0;
    for (const waiters of this.waiting.values())
      for (const resolve of waiters) resolve(null);
    this.waiting.clear();
  }

  /** `remember` faux : l'échec est rendu aux demandeurs sans être gardé (relu ensuite). */
  private finish(uuid: string, url: string | null, remember = true): void {
    if (remember) this.done.set(uuid, url);
    for (const resolve of this.waiting.get(uuid) ?? []) resolve(url);
    this.waiting.delete(uuid);
  }

  private pump(): void {
    while (!this.paused && this.running < THUMB_CONCURRENCY) {
      // Dernière demandée, première lue : ce sont les lignes visibles à l'instant (celles
      // qu'on a fait défiler sont déjà derrière).
      const uuid = this.queue.pop();
      if (uuid === undefined) return;
      this.running += 1;
      void this.load(uuid);
    }
  }

  private async load(uuid: string): Promise<void> {
    const epoch = this.epoch;
    let retry = false;
    try {
      const reply = await this.fetcher(this.root, uuid);
      if (reply.busy) {
        retry = true;
      } else {
        const url = reply.thumb?.startsWith("data:image/") ? reply.thumb : null;
        this.finish(uuid, url);
      }
    } catch {
      // Accroc ponctuel (clé USB) : pas de vignette pour cette demande, pas pour la session.
      this.finish(uuid, null, false);
    } finally {
      this.running -= 1;
    }
    if (retry) {
      await delay(DETAILS_RETRY_MS);
      if (this.epoch !== epoch) return;
      this.queue.push(uuid);
    }
    this.pump();
  }
}

/**
 * Vignette de l'histoire `uuid`, demandée quand sa ligne `ref` approche de la zone visible
 * (aussitôt si `IntersectionObserver` manque). `undefined` : pas encore lue ; `null` : la Flam
 * n'en a pas (ou elle est illisible).
 */
export function useDeviceThumbnail(
  loader: ThumbnailLoader | null,
  uuid: string,
  ref: RefObject<Element | null>,
): string | null | undefined {
  const [thumb, setThumb] = useState<{
    loader: ThumbnailLoader;
    uuid: string;
    url: string | null;
  }>();
  const known = loader?.peek(uuid);
  useEffect(() => {
    if (!loader || loader.peek(uuid) !== undefined) return;
    let active = true;
    const load = () => {
      void loader.request(uuid).then((url) => {
        if (active) setThumb({ loader, uuid, url });
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
  if (known !== undefined) return known;
  return thumb?.uuid === uuid && thumb.loader === loader ? thumb.url : undefined;
}
