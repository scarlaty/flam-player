import type { EpisodeRef } from "../../../api/pack";
import { RadioGroup, type RadioOption } from "../../../components";
import { t } from "../../../i18n";
import { sameEpisode } from "./tree";

export interface ImageCandidate {
  ref: EpisodeRef;
  title: string;
}

export interface CategoryImageChoiceProps {
  /** Nom affiché de la catégorie (annoncé avec le groupe). */
  categoryLabel: string;
  /** Épisodes de la catégorie (à toute profondeur) qui ont une pochette, dans l'ordre. */
  candidates: readonly ImageCandidate[];
  /** Image choisie ; `null` : automatique (image du premier épisode, N5 a). */
  value: EpisodeRef | null;
  onChange: (value: EpisodeRef | null) => void;
}

const AUTO = "auto";

/**
 * Image d'une catégorie (N5 c) : « Automatique » (par défaut) ou la pochette d'un des épisodes
 * qu'elle contient. Boutons radio natifs : une tabulation, flèches pour changer de choix. Une
 * seule pochette possible : rien à choisir, le composant ne s'affiche pas.
 */
export function CategoryImageChoice({
  categoryLabel,
  candidates,
  value,
  onChange,
}: CategoryImageChoiceProps) {
  const text = t.podcasts.pack.categoryImage;
  const index =
    value === null ? -1 : candidates.findIndex((c) => sameEpisode(c.ref, value));
  // Rien à choisir : un seul candidat, sans choix à défaire.
  if (candidates.length < 2 && value === null) return null;
  const options: RadioOption<string>[] = [
    { value: AUTO, label: text.auto, hint: text.autoHint },
    ...candidates.map((c, i) => ({ value: `e${i}`, label: c.title })),
  ];
  // Choix périmé (épisode qui n'est plus dans la catégorie) : affiché comme automatique.
  const selected = index < 0 ? AUTO : `e${index}`;
  return (
    <div className="fl-pack__image-choice">
      <RadioGroup
        legend={text.legend(categoryLabel)}
        options={options}
        value={selected}
        onChange={(next) => {
          if (next === AUTO) onChange(null);
          else onChange(candidates[Number(next.slice(1))]?.ref ?? null);
        }}
      />
    </div>
  );
}
