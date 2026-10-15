import type { ReactNode } from "react";
import { Icon, type IconName } from "../Icon";
import "./EmptyState.css";

export interface EmptyStateProps {
  icon?: IconName;
  title: string;
  description?: string;
  /** Action proposée (en général un `Button`). */
  action?: ReactNode;
  /** Niveau du titre, à ajuster selon la page (2 par défaut). */
  headingLevel?: 2 | 3;
}

export function EmptyState({
  icon = "sparkle",
  title,
  description,
  action,
  headingLevel = 2,
}: EmptyStateProps) {
  const Heading = headingLevel === 2 ? "h2" : "h3";
  return (
    <div className="fl-empty">
      <span className="fl-empty__icon">
        <Icon name={icon} size={36} />
      </span>
      <Heading className="fl-empty__title">{title}</Heading>
      {description && <p className="fl-empty__description">{description}</p>}
      {action && <div className="fl-empty__action">{action}</div>}
    </div>
  );
}
