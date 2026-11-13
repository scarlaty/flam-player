import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { t } from "../../i18n";
import { cx } from "../cx";
import { IconButton } from "../IconButton/IconButton";
import "./Dialog.css";

export interface DialogProps {
  open: boolean;
  /** Appelé sur Échap, clic sur le fond ou bouton Fermer. */
  onClose: () => void;
  title: string;
  /**
   * Phrase courte décrivant le dialogue, reliée par `aria-describedby`.
   * Sans elle, aucune description n'est annoncée (le contenu reste lisible normalement).
   */
  description?: string;
  /** `alertdialog` pour une confirmation qui demande une réponse (ex. suppression). */
  role?: "dialog" | "alertdialog";
  /** Panneau plus large (fiche détaillée). */
  wide?: boolean;
  /** Élément à focaliser à l'ouverture (ex. le bouton « Annuler »). */
  initialFocus?: RefObject<HTMLElement | null>;
  children?: ReactNode;
  /** Boutons d'action, alignés à droite en bas du dialogue. */
  footer?: ReactNode;
}

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * Boucle Tab et Maj+Tab dans `panel`. Seul le dialogue le plus récent (dernier panneau du
 * document) piège le focus. Focus hors du panneau, ou sur le panneau lui-même : on y entre
 * par le premier (Tab) ou le dernier (Maj+Tab) élément.
 */
function trapTab(event: globalThis.KeyboardEvent, panel: HTMLElement | null) {
  if (!panel) return;
  const panels = document.querySelectorAll(".fl-dialog");
  if (panels[panels.length - 1] !== panel) return;
  const focusable = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!first || !last) {
    event.preventDefault();
    panel.focus();
    return;
  }
  const active = document.activeElement;
  const inside = active !== null && panel.contains(active) && active !== panel;
  if (event.shiftKey && (!inside || active === first)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (!inside || active === last)) {
    event.preventDefault();
    first.focus();
  }
}

/**
 * Dialogue modal rendu dans `document.body`.
 * - Focus à l'ouverture : `initialFocus`, sinon premier élément actif du contenu, puis
 *   des actions, sinon le panneau lui-même ; rendu à l'élément d'origine à la fermeture.
 * - Tab et Maj+Tab bouclent dans le panneau, focus tombé sur `body` compris.
 * - Échap ferme, écouté sur tout le document tant que le dialogue est ouvert.
 * - Le défilement de la page est bloqué tant que le dialogue est ouvert.
 */
export function Dialog({
  open,
  onClose,
  title,
  description,
  role = "dialog",
  wide = false,
  initialFocus,
  children,
  footer,
}: DialogProps) {
  const titleId = useId();
  const descriptionId = useId();
  const panelRef = useRef<HTMLDivElement>(null);

  // Focus initial et restitution du focus. `useLayoutEffect` : le focus est posé pendant le
  // commit, avant que le dialogue soit observable (`findBy…`) ; un effet passif le poserait
  // un tick plus tard.
  useLayoutEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    const first =
      initialFocus?.current ??
      panel?.querySelector(".fl-dialog__body")?.querySelector<HTMLElement>(FOCUSABLE) ??
      panel?.querySelector(".fl-dialog__footer")?.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? panel)?.focus();
    return () => previous?.focus();
  }, [open, initialFocus]);

  // Échap et Tab au niveau du document, même si le focus a quitté le panneau (un bouton devenu
  // `disabled` rend le focus à `body`) : Tab ramène alors dans le dialogue au lieu de laisser le
  // focus partir derrière.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      } else if (event.key === "Tab") {
        trapTab(event, panelRef.current);
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  // Blocage du défilement de l'arrière-plan.
  useEffect(() => {
    if (!open) return;
    const { body } = document;
    const previous = body.style.overflow;
    body.style.overflow = "hidden";
    return () => {
      body.style.overflow = previous;
    };
  }, [open]);

  if (!open) return null;

  return createPortal(
    <div
      className="fl-dialog__backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        className={cx("fl-dialog", wide && "fl-dialog--wide")}
        role={role}
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        tabIndex={-1}
      >
        <div className="fl-dialog__header">
          <h2 id={titleId} className="fl-dialog__title">
            {title}
          </h2>
          <IconButton icon="close" label={t.common.close} onClick={onClose} />
        </div>
        {description && (
          <p id={descriptionId} className="fl-dialog__description">
            {description}
          </p>
        )}
        {children && <div className="fl-dialog__body">{children}</div>}
        {footer && <div className="fl-dialog__footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}
