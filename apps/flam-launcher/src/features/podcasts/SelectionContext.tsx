import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
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
  isSelected: (id: string) => boolean;
  /** Pourquoi cet épisode ne peut pas être ajouté ; `null` s'il le peut (ou s'il est déjà retenu). */
  refusal: (candidate: Pick<SelectedEpisode, "id" | "bytes">) => Refusal | null;
  /** Ajoute (si les plafonds le permettent) ou retire l'épisode. Renvoie `false` si refusé. */
  toggle: (episode: SelectedEpisode) => boolean;
  remove: (id: string) => void;
  clear: () => void;
}

const SelectionContext = createContext<PodcastSelection | null>(null);

export function PodcastSelectionProvider({ children }: { children: ReactNode }) {
  const [selected, setSelected] = useState<readonly SelectedEpisode[]>([]);

  const toggle = useCallback(
    (episode: SelectedEpisode) => {
      if (selected.some((s) => s.id === episode.id)) {
        setSelected((list) => list.filter((s) => s.id !== episode.id));
        return true;
      }
      if (refusalFor(selected, episode) !== null) return false;
      setSelected((list) => [...list, episode]);
      return true;
    },
    [selected],
  );

  const value = useMemo<PodcastSelection>(
    () => ({
      selected,
      totals: totalsOf(selected),
      isSelected: (id) => selected.some((s) => s.id === id),
      refusal: (candidate) =>
        selected.some((s) => s.id === candidate.id)
          ? null
          : refusalFor(selected, candidate),
      toggle,
      remove: (id) => setSelected((list) => list.filter((s) => s.id !== id)),
      clear: () => setSelected([]),
    }),
    [selected, toggle],
  );

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
