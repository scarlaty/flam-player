import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type ReactNode,
} from "react";
import { t } from "../../i18n";
import { cx } from "../cx";
import { Icon, type IconName } from "../Icon";
import { IconButton } from "../IconButton/IconButton";
import {
  ToastContext,
  type ToastApi,
  type ToastOptions,
  type ToastTone,
} from "./ToastContext";
import "./Toast.css";

export const DEFAULT_TOAST_DURATION = 5000;

const TONE_ICONS: Record<ToastTone, IconName> = {
  info: "info",
  success: "check",
  danger: "alert",
};

export interface ToastProps {
  message: string;
  tone?: ToastTone;
  onClose: () => void;
  /** Appelé quand le survol ou le focus commence (`true`) ou se termine (`false`). */
  onPauseChange?: (paused: boolean) => void;
}

/**
 * Notification seule, sans rôle ARIA : l'annonce est portée par la région qui la
 * contient (voir `ToastProvider`). En pratique affichée par `ToastProvider`.
 */
export function Toast({ message, tone = "info", onClose, onPauseChange }: ToastProps) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const paused = hovered || focused;

  useEffect(() => {
    onPauseChange?.(paused);
  }, [paused, onPauseChange]);

  function handleBlur(event: FocusEvent<HTMLDivElement>) {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null))
      setFocused(false);
  }

  return (
    <div
      className={cx("fl-toast", `fl-toast--${tone}`)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={handleBlur}
    >
      <span className="fl-toast__icon">
        <Icon name={TONE_ICONS[tone]} />
      </span>
      <p className="fl-toast__message">{message}</p>
      <IconButton icon="close" size="sm" label={t.common.close} onClick={onClose} />
    </div>
  );
}

interface ToastItem {
  id: number;
  message: string;
  tone: ToastTone;
  /** 0 = jusqu'à fermeture manuelle. */
  duration: number;
}

/** Notification avec minuterie, mise en pause pendant le survol ou le focus. */
function TimedToast({
  toast,
  onDismiss,
}: {
  toast: ToastItem;
  onDismiss: (id: number) => void;
}) {
  const [paused, setPaused] = useState(false);
  const remaining = useRef(toast.duration);
  const { id, duration } = toast;

  useEffect(() => {
    if (paused || duration <= 0) return;
    const start = Date.now();
    const timer = setTimeout(() => onDismiss(id), remaining.current);
    return () => {
      clearTimeout(timer);
      remaining.current -= Date.now() - start;
    };
  }, [paused, duration, id, onDismiss]);

  return (
    <Toast
      message={toast.message}
      tone={toast.tone}
      onClose={() => onDismiss(id)}
      onPauseChange={setPaused}
    />
  );
}

/**
 * Fournit `useToast()` et affiche les notifications en bas à droite.
 * - Infos et succès : région `aria-live="polite"`, disparition après `duration`
 *   (minuterie en pause au survol et au focus).
 * - Erreurs : région `aria-live="assertive"` séparée, affichage jusqu'à fermeture.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((toast) => toast.id !== id));
  }, []);

  const show = useCallback(
    ({ message, tone = "info", duration = DEFAULT_TOAST_DURATION }: ToastOptions) => {
      const id = nextId.current++;
      const lasting = tone === "danger" ? 0 : duration;
      setToasts((list) => [...list, { id, message, tone, duration: lasting }]);
      return id;
    },
    [],
  );

  const api = useMemo<ToastApi>(() => ({ show, dismiss }), [show, dismiss]);
  const render = (toast: ToastItem) => (
    <TimedToast key={toast.id} toast={toast} onDismiss={dismiss} />
  );

  return (
    <ToastContext.Provider value={api}>
      {children}
      <section className="fl-toasts" aria-label={t.toast.region}>
        <div className="fl-toasts__stack" aria-live="polite">
          {toasts.filter((toast) => toast.tone !== "danger").map(render)}
        </div>
        <div className="fl-toasts__stack" aria-live="assertive">
          {toasts.filter((toast) => toast.tone === "danger").map(render)}
        </div>
      </section>
    </ToastContext.Provider>
  );
}
