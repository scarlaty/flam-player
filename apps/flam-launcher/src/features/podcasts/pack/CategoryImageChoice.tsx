import { useEffect, useRef, useState } from "react";
import { episodeThumbnail, type EpisodeRef } from "../../../api/pack";
import { RadioGroup, type RadioOption } from "../../../components";
import { t } from "../../../i18n";
import { sameEpisode } from "./tree";

export interface ImageCandidate {
  ref: EpisodeRef;
  title: string;
}

/** Aperçu d'une pochette : URL `data:` ou `null` (pas d'aperçu). */
export type ThumbnailLoader = (ref: EpisodeRef) => Promise<string | null>;

export interface CategoryImageChoiceProps {
  /** Nom affiché de la catégorie (annoncé avec le groupe). */
  categoryLabel: string;
  /** Épisodes de la catégorie (à toute profondeur) qui ont une pochette, dans l'ordre. */
  candidates: readonly ImageCandidate[];
  /** Image choisie ; `null` : automatique (image du premier épisode, N5 a). */
  value: EpisodeRef | null;
  onChange: (value: EpisodeRef | null) => void;
  /** Lecture d'un aperçu (par défaut : backend, cache puis téléchargement à la demande, #508). */
  thumbnailOf?: ThumbnailLoader;
}

const AUTO = "auto";

/**
 * Vignette d'un épisode, demandée au backend (qui la télécharge au besoin) quand elle approche
 * de l'écran : un épisode hors écran ne déclenche aucun téléchargement. Sans aperçu (pas de
 * pochette, hors ligne, erreur), rien n'est affiché : le libellé du choix suffit. L'image est
 * décorative (`alt=""`) : le libellé du radio porte déjà le titre.
 */
function EpisodeThumb({ episode, load }: { episode: EpisodeRef; load: ThumbnailLoader }) {
  const holder = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(typeof IntersectionObserver === "undefined");
  const [src, setSrc] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    const el = holder.current;
    if (visible || !el) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        setVisible(true);
        observer.disconnect();
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [visible]);
  const { source_id, episode_id } = episode;
  useEffect(() => {
    if (!visible) return;
    let cancelled = false;
    load({ source_id, episode_id })
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setSrc(null);
      });
    return () => {
      cancelled = true;
    };
  }, [visible, load, source_id, episode_id]);
  if (src === null) return null;
  return (
    <span ref={holder} className="fl-pack__thumb">
      {src && <img src={src} alt="" width={48} height={36} />}
    </span>
  );
}

/**
 * Image d'une catégorie (N5 c) : « Automatique » (par défaut) ou la pochette d'un des épisodes
 * qu'elle contient, avec un aperçu de chaque pochette. Boutons radio natifs : une tabulation,
 * flèches pour changer de choix. Une seule pochette possible : rien à choisir, le composant ne
 * s'affiche pas.
 */
export function CategoryImageChoice({
  categoryLabel,
  candidates,
  value,
  onChange,
  thumbnailOf = episodeThumbnail,
}: CategoryImageChoiceProps) {
  const text = t.podcasts.pack.categoryImage;
  const index =
    value === null ? -1 : candidates.findIndex((c) => sameEpisode(c.ref, value));
  // Rien à choisir : un seul candidat, sans choix à défaire.
  if (candidates.length < 2 && value === null) return null;
  // Deux épisodes de même titre : le rang (dans la catégorie) les distingue.
  const titleCount = new Map<string, number>();
  for (const c of candidates) titleCount.set(c.title, (titleCount.get(c.title) ?? 0) + 1);
  const options: RadioOption<string>[] = [
    { value: AUTO, label: text.auto, hint: text.autoHint },
    ...candidates.map((c, i) => ({
      value: `e${i}`,
      label: (titleCount.get(c.title) ?? 0) > 1 ? text.ranked(c.title, i + 1) : c.title,
      media: (
        <EpisodeThumb
          key={`${c.ref.source_id}/${c.ref.episode_id}`}
          episode={c.ref}
          load={thumbnailOf}
        />
      ),
    })),
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
