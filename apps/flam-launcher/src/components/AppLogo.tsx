import "./AppLogo.css";

/**
 * Logo de l'application : une petite flamme sur un carré orange brique arrondi.
 * Même dessin que l'icône de l'application (`src-tauri/icons/app-icon.svg`).
 */
export function AppLogo({ size = 32 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      aria-hidden="true"
      focusable="false"
    >
      <rect width="64" height="64" rx="16" className="fl-logo__bg" />
      <path
        d="M32 9c5 10 15 16 15 30a15 15 0 0 1-30 0c0-8 4-12 7-18 1 6 4 9 7 9-2-7-2-14 1-21z"
        className="fl-logo__flame"
      />
      <path
        d="M32 33c3 4 7 6 7 11a7 7 0 0 1-14 0c0-5 4-7 7-11z"
        className="fl-logo__core"
      />
    </svg>
  );
}
