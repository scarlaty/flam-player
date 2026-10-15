import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { t } from "../../i18n";
import { IconButton } from "../IconButton/IconButton";
import "./Dialog.css";

export interface DialogProps {
  open: boolean;
  /** Appelé sur Échap, clic sur le fond ou bouton Fermer. */
  onClose: () => void;
  title: string;
  children: ReactNode;
  /** Boutons d'action, alignés à droite en bas du dialogue. */
  footer?: ReactNode;
}

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * Dialogue modal : focus placé dans le dialogue à l'ouverture, Tab confiné,
 * Échap pour fermer, focus rendu à l'élément d'origine à la fermeture.
 */
export function Dialog({ open, onClose, title, children, footer }: DialogProps) {
  const titleId = useId();
  const bodyId = useId();
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    // Premier élément actif du contenu, sinon des actions, sinon le dialogue lui-même.
    const first =
      panel?.querySelector(".fl-dialog__body")?.querySelector<HTMLElement>(FOCUSABLE) ??
      panel?.querySelector(".fl-dialog__footer")?.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? panel)?.focus();
    return () => previous?.focus();
  }, [open]);

  if (!open) return null;

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== "Tab" || !panelRef.current) return;
    const focusable = Array.from(
      panelRef.current.querySelectorAll<HTMLElement>(FOCUSABLE),
    );
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return createPortal(
    <div
      className="fl-dialog__backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        className="fl-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={bodyId}
        tabIndex={-1}
        onKeyDown={handleKeyDown}
      >
        <div className="fl-dialog__header">
          <h2 id={titleId} className="fl-dialog__title">
            {title}
          </h2>
          <IconButton icon="close" label={t.common.close} onClick={onClose} />
        </div>
        <div id={bodyId} className="fl-dialog__body">
          {children}
        </div>
        {footer && <div className="fl-dialog__footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}
