import { useId, useState, type CSSProperties, type ReactNode } from "react";
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
   * Explication du badge. Un seul mécanisme : le badge devient focalisable (Tab) et affiche une
   * info-bulle (`role="tooltip"`) au survol ou au focus, fermable par Échap ; le même élément sert de
   * description accessible (`aria-describedby`), lue une seule fois, hors du nom du badge.
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
  const tooltipId = useId();
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<CSSProperties>();

  const classes = cx("fl-badge", `fl-badge--${tone}`, className);
  const content = (
    <>
      {icon && <Icon name={icon} size={14} />}
      {children}
    </>
  );
  if (!description) {
    return <span className={classes}>{content}</span>;
  }

  const show = (target: HTMLElement) => {
    const rect = target.getBoundingClientRect();
    // Position fixe : une carte à `overflow: hidden` ne doit pas tronquer l'info-bulle.
    setPosition({
      top: rect.bottom + 4,
      left: Math.max(8, Math.min(rect.left, window.innerWidth - 288)),
    });
    setOpen(true);
  };

  return (
    <>
      <span
        className={cx(classes, "fl-badge--described")}
        tabIndex={0}
        aria-describedby={tooltipId}
        onMouseEnter={(event) => show(event.currentTarget)}
        onMouseLeave={() => setOpen(false)}
        onFocus={(event) => show(event.currentTarget)}
        onBlur={() => setOpen(false)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && open) {
            event.stopPropagation();
            setOpen(false);
          }
        }}
      >
        {content}
      </span>
      <span
        id={tooltipId}
        role="tooltip"
        className="fl-badge__tooltip"
        style={position}
        hidden={!open}
      >
        {description}
      </span>
    </>
  );
}
