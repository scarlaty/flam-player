import type { ReactNode } from "react";
import { t } from "../../i18n";
import { Badge, type BadgeTone } from "../Badge/Badge";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./StoryCard.css";

export interface StoryBadge {
  label: string;
  tone?: BadgeTone;
  icon?: IconName;
}

export interface StoryCardProps {
  title: string;
  /** Adresse de la vignette ; absente = illustration générique. */
  coverUrl?: string;
  ageMin?: number;
  ageMax?: number;
  badges?: StoryBadge[];
  /** Carte sélectionnée (fiche ouverte). */
  selected?: boolean;
  /** Ouvre la fiche de l'histoire (clic ou Entrée sur le titre). */
  onOpen?: () => void;
  /** Actions rapides (ex. bouton Jouer), placées au-dessus de la zone cliquable. */
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
  actions,
}: StoryCardProps) {
  return (
    <article className={cx("fl-story-card", selected && "fl-story-card--selected")}>
      <div className="fl-story-card__cover">
        {coverUrl ? (
          <img src={coverUrl} alt="" loading="lazy" />
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
              <li key={badge.label}>
                <Badge tone={badge.tone} icon={badge.icon}>
                  {badge.label}
                </Badge>
              </li>
            ))}
          </ul>
        )}
      </div>
      {actions && <div className="fl-story-card__actions">{actions}</div>}
    </article>
  );
}
