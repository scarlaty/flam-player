import type { ReactNode } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./Badge.css";

export type BadgeTone = "neutral" | "info" | "success" | "warning" | "danger";

export interface BadgeProps {
  tone?: BadgeTone;
  icon?: IconName;
  children: ReactNode;
  className?: string;
}

/** Pastille d'état courte. Le sens n'est jamais porté par la seule couleur : texte obligatoire. */
export function Badge({ tone = "neutral", icon, children, className }: BadgeProps) {
  return (
    <span className={cx("fl-badge", `fl-badge--${tone}`, className)}>
      {icon && <Icon name={icon} size={14} />}
      {children}
    </span>
  );
}
