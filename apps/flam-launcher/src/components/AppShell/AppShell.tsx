import { useRef, type MouseEvent, type ReactNode } from "react";
import { t } from "../../i18n";
import { AppLogo } from "../AppLogo";
import "./AppShell.css";

export interface AppShellProps {
  /** Contenu de la barre latérale (en général `SidebarSourceList`). */
  sidebar: ReactNode;
  /** Titre de la vue courante, affiché dans l'en-tête (niveau h1). */
  title: ReactNode;
  /** Actions de l'en-tête (recherche, boutons…). */
  headerActions?: ReactNode;
  /** Panneau latéral droit facultatif (en général `TaskPanel`). */
  aside?: ReactNode;
  children: ReactNode;
}

const MAIN_ID = "fl-main";

/** Mise en page principale : barre latérale, en-tête, contenu et panneau facultatif. */
export function AppShell({
  sidebar,
  title,
  headerActions,
  aside,
  children,
}: AppShellProps) {
  const mainRef = useRef<HTMLElement>(null);

  // Focalise le contenu sans modifier le hash : il sert au routage (#/demo…).
  function skipToMain(event: MouseEvent<HTMLAnchorElement>) {
    event.preventDefault();
    mainRef.current?.focus();
  }

  return (
    <div className="fl-shell">
      <a className="fl-shell__skip" href={`#${MAIN_ID}`} onClick={skipToMain}>
        {t.app.skipToContent}
      </a>
      <nav className="fl-shell__sidebar" aria-label={t.app.sourcesNav}>
        <div className="fl-shell__brand">
          <AppLogo size={32} />
          <span>{t.app.name}</span>
        </div>
        {sidebar}
      </nav>
      <div className="fl-shell__content">
        <header className="fl-shell__header">
          <h1 className="fl-shell__title">{title}</h1>
          {headerActions && <div className="fl-shell__actions">{headerActions}</div>}
        </header>
        <main ref={mainRef} id={MAIN_ID} className="fl-shell__main" tabIndex={-1}>
          {children}
        </main>
      </div>
      {aside && <aside className="fl-shell__aside">{aside}</aside>}
    </div>
  );
}
