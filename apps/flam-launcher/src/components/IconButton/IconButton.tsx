import type { ComponentPropsWithRef } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import "./IconButton.css";

export interface IconButtonProps extends Omit<
  ComponentPropsWithRef<"button">,
  "children" | "aria-label"
> {
  icon: IconName;
  /** Libellé accessible, également affiché en infobulle. Obligatoire. */
  label: string;
  variant?: "ghost" | "secondary" | "primary";
  size?: "md" | "sm";
}

export function IconButton({
  icon,
  label,
  variant = "ghost",
  size = "md",
  type = "button",
  className,
  title,
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      title={title ?? label}
      className={cx(
        "fl-icon-button",
        `fl-icon-button--${variant}`,
        `fl-icon-button--${size}`,
        className,
      )}
      {...rest}
    >
      <Icon name={icon} size={size === "sm" ? 16 : 20} />
    </button>
  );
}
