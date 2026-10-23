import { useState, type ReactNode } from "react";
import { t } from "../../i18n";
import { Badge, type BadgeTone } from "../Badge/Badge";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import { ProgressBar } from "../ProgressBar/ProgressBar";
import "./StoryCard.css";

export interface StoryBadge {
  /** Identifiant stable du badge (ex. « installed »), utilisé comme clé. */
  id: string;
  label: string;
  tone?: BadgeTone;
  icon?: IconName;
  /** Précision visible sous le badge (ex. la raison d'une incompatibilité). */
  detail?: string;
  /** Info-bulle et texte accessible expliquant le badge. */
  description?: string;
}

export interface StoryCardProps {
  title: string;
  /** Adresse de la vignette ; absente ou illisible = illustration générique. */
  coverUrl?: string;
  ageMin?: number;
  ageMax?: number;
  badges?: StoryBadge[];
  /** Carte sélectionnée (fiche ouverte). */
  selected?: boolean;
  /** Ouvre la fiche de l'histoire (clic ou Entrée sur le titre). */
  onOpen?: () => void;
  /**
   * Opération en cours sur l'histoire : pourcentage 0-100, ou `"indeterminate"`.
   * Affiche une barre de progression sous les badges.
   */
  progress?: number | "indeterminate";
  /** Texte visible sous la barre (ex. « Conversion… »). */
  progressText?: string;
  /**
   * Actions rapides (ex. Jouer, Envoyer, Supprimer), dans une rangée en bas de la carte : dans le
   * flux, au-dessus de la zone cliquable, sans recouvrir la vignette ni les badges.
   */
  actions?: ReactNode;
}

const PLACEHOLDER_TONES = ["info", "success", "warning", "primary"] as const;

/** Couleur d'illustration stable pour un titre donné. */
export function placeholderTone(title: string): (typeof PLACEHOLDER_TONES)[number] {
  let hash = 0;
  for (const char of title) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return PLACEHOLDER_TONES[hash % PLACEHOLDER_TONES.length] ?? "primary";
}

export function StoryCard({
  title,
  coverUrl,
  ageMin,
  ageMax,
  badges = [],
  selected = false,
  onOpen,
  progress,
  progressText,
  actions,
}: StoryCardProps) {
  // Adresse dont le chargement a échoué : on affiche alors l'illustration générique.
  const [failedUrl, setFailedUrl] = useState<string>();
  const showCover = coverUrl !== undefined && coverUrl !== failedUrl;

  return (
    <article className={cx("fl-story-card", selected && "fl-story-card--selected")}>
      <div className="fl-story-card__cover">
        {showCover ? (
          <img
            src={coverUrl}
            alt=""
            loading="lazy"
            onError={() => setFailedUrl(coverUrl)}
          />
        ) : (
          <div
            className={`fl-story-card__placeholder fl-story-card__placeholder--${placeholderTone(title)}`}
          >
            <Icon name="book" size={40} />
          </div>
        )}
      </div>
      <div className="fl-story-card__body">
        <h3 className="fl-story-card__title">
          {onOpen ? (
            <button
              type="button"
              className="fl-story-card__open"
              onClick={onOpen}
              aria-current={selected ? "true" : undefined}
            >
              {title}
            </button>
          ) : (
            title
          )}
        </h3>
        {ageMin !== undefined && (
          <p className="fl-story-card__age">{t.story.age(ageMin, ageMax)}</p>
        )}
        {badges.length > 0 && (
          <ul className="fl-story-card__badges">
            {badges.map((badge) => (
              <li key={badge.id}>
                <Badge
                  tone={badge.tone}
                  icon={badge.icon}
                  description={badge.description}
                >
                  {badge.label}
                </Badge>
                {badge.detail && (
                  <p className="fl-story-card__badge-detail">{badge.detail}</p>
                )}
              </li>
            ))}
          </ul>
        )}
        {progress !== undefined && (
          <div className="fl-story-card__progress">
            <ProgressBar
              value={progress === "indeterminate" ? undefined : progress}
              label={t.story.progress(title)}
            />
            {progressText && (
              <p className="fl-story-card__progress-text">{progressText}</p>
            )}
          </div>
        )}
      </div>
      {actions && <div className="fl-story-card__actions">{actions}</div>}
    </article>
  );
}
