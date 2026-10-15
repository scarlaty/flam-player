import type { ComponentPropsWithRef } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./Button.css";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "md" | "sm";

export interface ButtonProps extends ComponentPropsWithRef<"button"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Icône affichée avant le libellé. */
  icon?: IconName;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  type = "button",
  className,
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(
        "fl-button",
        `fl-button--${variant}`,
        `fl-button--${size}`,
        className,
      )}
      {...rest}
    >
      {icon && <Icon name={icon} size={size === "sm" ? 16 : 18} />}
      <span>{children}</span>
    </button>
  );
}
