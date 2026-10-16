import { useId, type ComponentPropsWithRef, type ReactNode } from "react";
import { cx } from "../cx";
import "./TextField.css";

export interface TextFieldProps extends Omit<ComponentPropsWithRef<"input">, "id"> {
  /** Libellé visible, relié au champ. Obligatoire. */
  label: string;
  /** Aide affichée sous le champ (format attendu, valeur utilisée…). */
  hint?: ReactNode;
  /** Message d'erreur ; le champ est alors marqué invalide. */
  error?: string | null;
  /** Boutons placés à droite du champ (ex. « Parcourir… »). */
  actions?: ReactNode;
}

/**
 * Champ texte accessible : libellé, aide et erreur reliés par `aria-describedby`.
 * L'erreur est annoncée poliment (région `aria-live` toujours présente) quand elle change.
 */
export function TextField({
  label,
  hint,
  error,
  actions,
  className,
  ...rest
}: TextFieldProps) {
  const id = useId();
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  const describedBy = [hint ? hintId : null, error ? errorId : null]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cx("fl-field", className)}>
      <label className="fl-field__label" htmlFor={id}>
        {label}
      </label>
      <div className="fl-field__row">
        <input
          id={id}
          type="text"
          className={cx("fl-field__input", error && "fl-field__input--invalid")}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy || undefined}
          {...rest}
        />
        {actions && <div className="fl-field__actions">{actions}</div>}
      </div>
      {hint && (
        <p id={hintId} className="fl-field__hint">
          {hint}
        </p>
      )}
      <div aria-live="polite">
        {error && (
          <p id={errorId} className="fl-field__error">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
