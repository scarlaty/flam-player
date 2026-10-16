import "./AppLogo.css";

/**
 * Logo de l'application : un petit lecteur d'histoires (boîtier vert d'eau, écran avec une
 * flamme, molette jaune, touches orange) sur un carré vert arrondi.
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
      <rect x="17" y="7" width="30" height="50" rx="6" className="fl-logo__body" />
      <rect x="21" y="11" width="22" height="16" rx="2.5" className="fl-logo__screen" />
      <path
        d="M32 14c2 3 4.5 4.5 4.5 8a4.5 4.5 0 0 1-9 0c0-2 1-3 2-4.5.4 1.5 1 2 1.5 2-.5-2-.5-3.5 1-5.5z"
        className="fl-logo__flame"
      />
      <circle cx="28.5" cy="43" r="8.5" className="fl-logo__wheel" />
      <circle cx="28.5" cy="43" r="3.5" className="fl-logo__hub" />
      <circle cx="41" cy="36.5" r="3.2" className="fl-logo__key" />
      <circle cx="41" cy="47.5" r="3.2" className="fl-logo__key" />
    </svg>
  );
}
