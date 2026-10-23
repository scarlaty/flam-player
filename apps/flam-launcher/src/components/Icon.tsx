import type { ReactNode } from "react";

/*
 * Icônes originales dessinées pour flam-launcher (grille 24 × 24, trait de 2 px,
 * extrémités arrondies). Elles sont décoratives : le libellé accessible est porté
 * par le bouton ou le texte voisin.
 */
const paths = {
  library: (
    <>
      <path d="M5 4h3v16H5z" />
      <path d="M10 4h3v16h-3z" />
      <path d="m15.5 5 2.9-.8 3.1 15-2.9.8z" />
    </>
  ),
  catalog: (
    <>
      <path d="M7 18a4.5 4.5 0 0 1-.6-9A6 6 0 0 1 18 9.5a4 4 0 0 1-.5 8.5z" />
      <path d="M12 11v6m-2.5-2.5L12 17l2.5-2.5" />
    </>
  ),
  folder: (
    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
  ),
  offline: (
    <>
      <path d="M7 18a4.5 4.5 0 0 1-.6-9A6 6 0 0 1 18 9.5a4 4 0 0 1-.5 8.5z" />
      <path d="m4 4 16 16" />
    </>
  ),
  install: (
    <>
      <path d="m4 8 8-4 8 4v8l-8 4-8-4z" />
      <path d="m4 8 8 4 8-4M12 12v8" />
    </>
  ),
  pencil: <path d="M5 19h3.5L19 8.5 15.5 5 5 15.5zM13.5 7l3.5 3.5" />,
  arrowUp: <path d="M12 19V5m-5.5 5.5L12 5l5.5 5.5" />,
  arrowDown: <path d="M12 5v14m-5.5-5.5L12 19l5.5-5.5" />,
  arrowTop: <path d="M5 4.5h14M12 20V9m-5 4 5-5 5 5" />,
  arrowBottom: <path d="M5 19.5h14M12 4v11m-5-4 5 5 5-5" />,
  plus: <path d="M12 5v14M5 12h14" />,
  close: <path d="m6 6 12 12M18 6 6 18" />,
  download: <path d="M12 4v11m-4.5-4.5L12 15l4.5-4.5M5 19h14" />,
  play: (
    <path d="M8 5.5v13a1 1 0 0 0 1.5.9l10-6.5a1 1 0 0 0 0-1.8l-10-6.5A1 1 0 0 0 8 5.5z" />
  ),
  refresh: (
    <>
      <path d="M19 12a7 7 0 1 1-2-4.9" />
      <path d="M19 4v4h-4" />
    </>
  ),
  device: (
    <>
      <rect x="5" y="4" width="14" height="16" rx="3" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  sliders: (
    <>
      <path d="M4 7h9m4 0h3M4 17h3m4 0h9" />
      <circle cx="15" cy="7" r="2" />
      <circle cx="9" cy="17" r="2" />
    </>
  ),
  trash: <path d="M4 7h16M9 7V4h6v3m-9 0 1 13h10l1-13M10 11v6m4-6v6" />,
  check: <path d="m5 12.5 4.5 4.5L19 7.5" />,
  alert: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5v5.5m0 3.5v.01" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5.5m0-9v.01" />
    </>
  ),
  book: (
    <>
      <path d="M12 6.5C10 5 7 4.5 4 5v13c3-.5 6 0 8 1.5 2-1.5 5-2 8-1.5V5c-3-.5-6 0-8 1.5z" />
      <path d="M12 6.5v13" />
    </>
  ),
  sparkle: (
    <path d="M12 3c.7 4.5 2.5 6.3 7 7-4.5.7-6.3 2.5-7 7-.7-4.5-2.5-6.3-7-7 4.5-.7 6.3-2.5 7-7z" />
  ),
  podcast: (
    <>
      <path d="M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3z" />
      <path d="M6 11a6 6 0 0 0 12 0M12 17v4M9 21h6" />
    </>
  ),
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof paths;

export interface IconProps {
  name: IconName;
  /** Taille en pixels (carré). */
  size?: number;
  className?: string;
}

export function Icon({ name, size = 20, className }: IconProps) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {paths[name]}
    </svg>
  );
}
