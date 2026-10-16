import { useId, type InputHTMLAttributes } from "react";
import "./CheckboxField.css";

export interface CheckboxFieldProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "type" | "children"
> {
  label: string;
  /** Précision affichée sous le libellé et reliée par `aria-describedby`. */
  hint?: string;
}

/** Case à cocher avec libellé cliquable et précision optionnelle. */
export function CheckboxField({ label, hint, id, ...rest }: CheckboxFieldProps) {
  const generated = useId();
  const inputId = id ?? generated;
  const hintId = `${inputId}-hint`;
  return (
    <div className="fl-check">
      <input
        {...rest}
        id={inputId}
        type="checkbox"
        aria-describedby={hint ? hintId : undefined}
      />
      <div>
        <label htmlFor={inputId}>{label}</label>
        {hint && (
          <p id={hintId} className="fl-check__hint">
            {hint}
          </p>
        )}
      </div>
    </div>
  );
}
