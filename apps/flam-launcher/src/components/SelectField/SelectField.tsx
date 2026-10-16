import { useId, type ComponentPropsWithRef } from "react";
import { cx } from "../cx";
import "./SelectField.css";

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectFieldProps extends Omit<
  ComponentPropsWithRef<"select">,
  "id" | "children"
> {
  /** Libellé visible, relié à la liste. Obligatoire. */
  label: string;
  options: SelectOption[];
}

/** Liste déroulante native avec libellé visible (accessible au clavier et aux lecteurs d'écran). */
export function SelectField({ label, options, className, ...rest }: SelectFieldProps) {
  const id = useId();
  return (
    <div className={cx("fl-select", className)}>
      <label className="fl-select__label" htmlFor={id}>
        {label}
      </label>
      <select id={id} className="fl-select__input" {...rest}>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  );
}
