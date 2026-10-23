import type { ReactNode } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./Banner.css";

export type BannerTone = "info" | "success" | "warning" | "danger";

export interface BannerProps {
  tone?: BannerTone;
  /** Titre court en gras (facultatif). */
  title?: string;
  children?: ReactNode;
  /** Action proposée à droite (en général un `Button`). */
  action?: ReactNode;
  /**
   * Rôle ARIA : `alert` (annonce immédiate) par défaut pour `danger`, `status` sinon ; `none`
   * quand une autre région vivante annonce déjà le même message (pas de lecture en double).
   */
  role?: "alert" | "status" | "none";
  className?: string;
}

const TONE_ICONS: Record<BannerTone, IconName> = {
  info: "info",
  success: "check",
  warning: "alert",
  danger: "alert",
};

/** Message en bandeau (avertissement, erreur, information). Le ton n'est jamais seul : icône et texte. */
export function Banner({
  tone = "info",
  title,
  children,
  action,
  role,
  className,
}: BannerProps) {
  return (
    <div
      className={cx("fl-banner", `fl-banner--${tone}`, className)}
      role={
        role === "none" ? undefined : (role ?? (tone === "danger" ? "alert" : "status"))
      }
    >
      <span className="fl-banner__icon">
        <Icon name={TONE_ICONS[tone]} />
      </span>
      <div className="fl-banner__body">
        {title && <p className="fl-banner__title">{title}</p>}
        {children && <div className="fl-banner__text">{children}</div>}
      </div>
      {action && <div className="fl-banner__action">{action}</div>}
    </div>
  );
}
