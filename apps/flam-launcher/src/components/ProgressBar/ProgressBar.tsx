import { cx } from "../cx";
import "./ProgressBar.css";

export interface ProgressBarProps {
  /** Pourcentage de 0 à 100 ; absent = progression indéterminée. */
  value?: number;
  /** Libellé accessible (lu par les lecteurs d'écran). */
  label: string;
  /** Affiche le pourcentage à droite de la barre. */
  showValue?: boolean;
  tone?: "primary" | "success" | "danger";
  className?: string;
}

export function ProgressBar({
  value,
  label,
  showValue = false,
  tone = "primary",
  className,
}: ProgressBarProps) {
  const determinate = value !== undefined;
  const percent = determinate ? Math.round(Math.min(100, Math.max(0, value))) : 0;

  return (
    <div className={cx("fl-progress", `fl-progress--${tone}`, className)}>
      <div
        className={cx(
          "fl-progress__track",
          !determinate && "fl-progress__track--indeterminate",
        )}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={determinate ? percent : undefined}
      >
        <div
          className="fl-progress__fill"
          style={determinate ? { inlineSize: `${percent}%` } : undefined}
        />
      </div>
      {showValue && determinate && (
        <span className="fl-progress__value" aria-hidden="true">
          {percent} %
        </span>
      )}
    </div>
  );
}
