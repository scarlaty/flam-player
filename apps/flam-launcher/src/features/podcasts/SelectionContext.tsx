import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  episodeKey,
  refusalFor,
  totalsOf,
  type Refusal,
  type SelectedEpisode,
  type Totals,
} from "./selection";

/**
 * Sélection d'épisodes mémorisée entre les écrans (lue par l'éditeur de pack, #363). Elle vit
 * en mémoire tant que l'application tourne ; le brouillon enregistré est l'affaire de #362.
 */
export interface PodcastSelection {
  /** Épisodes retenus, dans l'ordre de sélection. */
  selected: readonly SelectedEpisode[];
  totals: Totals;
  isSelected: (sourceId: string, id: string) => boolean;
  /** Pourquoi cet épisode ne peut pas être ajouté ; `null` s'il le peut (ou s'il est déjà retenu). */
  refusal: (
    candidate: Pick<SelectedEpisode, "id" | "sourceId" | "bytes">,
  ) => Refusal | null;
  /** Ajoute (si les plafonds le permettent) ou retire l'épisode ; un refus ne change rien. */
  toggle: (episode: SelectedEpisode) => void;
  /** Ajoute d'un coup les épisodes que les plafonds acceptent, dans l'ordre ; les autres sont ignorés. */
  addMany: (episodes: readonly SelectedEpisode[]) => void;
  remove: (sourceId: string, id: string) => void;
  clear: () => void;
}

const SelectionContext = createContext<PodcastSelection | null>(null);

export function PodcastSelectionProvider({ children }: { children: ReactNode }) {
  const [selected, setSelected] = useState<readonly SelectedEpisode[]>([]);

  // Mise à jour fonctionnelle : deux bascules rapprochées partent toutes deux de l'état à jour.
  const toggle = useCallback((episode: SelectedEpisode) => {
    const key = episodeKey(episode.sourceId, episode.id);
    setSelected((list) => {
      if (list.some((s) => episodeKey(s.sourceId, s.id) === key)) {
        return list.filter((s) => episodeKey(s.sourceId, s.id) !== key);
      }
      return refusalFor(list, episode) === null ? [...list, episode] : list;
    });
  }, []);

  const addMany = useCallback((episodes: readonly SelectedEpisode[]) => {
    setSelected((list) => {
      const next = [...list];
      for (const episode of episodes) {
        if (refusalFor(next, episode) === null) next.push(episode);
      }
      return next.length === list.length ? list : next;
    });
  }, []);

  const value = useMemo<PodcastSelection>(() => {
    const keys = new Set(selected.map((s) => episodeKey(s.sourceId, s.id)));
    return {
      selected,
      totals: totalsOf(selected),
      isSelected: (sourceId, id) => keys.has(episodeKey(sourceId, id)),
      refusal: (candidate) =>
        keys.has(episodeKey(candidate.sourceId, candidate.id))
          ? null
          : refusalFor(selected, candidate),
      toggle,
      addMany,
      remove: (sourceId, id) => {
        const key = episodeKey(sourceId, id);
        setSelected((list) => list.filter((s) => episodeKey(s.sourceId, s.id) !== key));
      },
      clear: () => setSelected([]),
    };
  }, [selected, toggle, addMany]);

  return <SelectionContext.Provider value={value}>{children}</SelectionContext.Provider>;
}

export function usePodcastSelection(): PodcastSelection {
  const value = useContext(SelectionContext);
  if (!value) {
    throw new Error(
      "usePodcastSelection doit être utilisé sous PodcastSelectionProvider",
    );
  }
  return value;
}
