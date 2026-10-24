import { useId, type ReactNode } from "react";
import "./RadioGroup.css";

export interface RadioOption<T extends string> {
  value: T;
  label: string;
  /** Précision affichée sous le libellé. */
  hint?: string;
  /** Illustration entre la case et le libellé (le libellé reste le nom du choix). */
  media?: ReactNode;
}

export interface RadioGroupProps<T extends string> {
  /** Question posée, lue avec chaque choix. */
  legend: string;
  options: readonly RadioOption<T>[];
  value: T;
  onChange: (value: T) => void;
  disabled?: boolean;
}

/**
 * Groupe de boutons radio natifs (une seule tabulation, flèches pour changer de choix),
 * regroupés dans un `fieldset` portant la question.
 */
export function RadioGroup<T extends string>({
  legend,
  options,
  value,
  onChange,
  disabled,
}: RadioGroupProps<T>) {
  const name = useId();
  return (
    <fieldset className="fl-radio-group" disabled={disabled}>
      <legend className="fl-radio-group__legend">{legend}</legend>
      <div className="fl-radio-group__options">
        {options.map((option) => (
          <label key={option.value} className="fl-radio">
            <input
              type="radio"
              name={name}
              value={option.value}
              checked={value === option.value}
              onChange={() => onChange(option.value)}
            />
            {option.media}
            <span className="fl-radio__text">
              <span>{option.label}</span>
              {option.hint && <span className="fl-radio__hint">{option.hint}</span>}
            </span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}
