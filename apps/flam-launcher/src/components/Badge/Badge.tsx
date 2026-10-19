import { useId, type ReactNode } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./Badge.css";

export type BadgeTone =
  "neutral" | "info" | "success" | "warning" | "danger" | "secondary";

export interface BadgeProps {
  tone?: BadgeTone;
  icon?: IconName;
  children: ReactNode;
  className?: string;
  /**
   * Explication du badge : info-bulle (`title`) et texte accessible (`aria-describedby` vers un
   * texte masqué), pour les lecteurs d'écran et le clavier.
   */
  description?: string;
}

/** Pastille d'état courte. Le sens n'est jamais porté par la seule couleur : texte obligatoire. */
export function Badge({
  tone = "neutral",
  icon,
  children,
  className,
  description,
}: BadgeProps) {
  const descriptionId = useId();
  return (
    <span
      className={cx("fl-badge", `fl-badge--${tone}`, className)}
      title={description}
      aria-describedby={description ? descriptionId : undefined}
    >
      {icon && <Icon name={icon} size={14} />}
      {children}
      {description && (
        <span id={descriptionId} className="fl-visually-hidden">
          {description}
        </span>
      )}
    </span>
  );
}
