import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
}

/** Notification seule ; en pratique affichée par `ToastProvider`. */
export function Toast({ message, tone = "info", onClose }: ToastProps) {
  return (
    <div
      className={cx("fl-toast", `fl-toast--${tone}`)}
      role={tone === "danger" ? "alert" : undefined}
    >
      <span className="fl-toast__icon">
        <Icon name={TONE_ICONS[tone]} />
      </span>
      <p className="fl-toast__message">{message}</p>
      <IconButton icon="close" size="sm" label={t.common.close} onClick={onClose} />
    </div>
  );
}

interface ToastItem extends Required<ToastOptions> {
  id: number;
}

/** Fournit `useToast()` et affiche la pile de notifications en bas à droite. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());

  const dismiss = useCallback((id: number) => {
    const timer = timers.current.get(id);
    if (timer) clearTimeout(timer);
    timers.current.delete(id);
    setToasts((list) => list.filter((toast) => toast.id !== id));
  }, []);

  const show = useCallback(
    ({ message, tone = "info", duration = DEFAULT_TOAST_DURATION }: ToastOptions) => {
      const id = nextId.current++;
      setToasts((list) => [...list, { id, message, tone, duration }]);
      if (duration > 0) {
        timers.current.set(
          id,
          setTimeout(() => dismiss(id), duration),
        );
      }
      return id;
    },
    [dismiss],
  );

  useEffect(() => {
    const pending = timers.current;
    return () => pending.forEach((timer) => clearTimeout(timer));
  }, []);

  const api = useMemo<ToastApi>(() => ({ show, dismiss }), [show, dismiss]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <section className="fl-toasts" aria-label={t.toast.region} aria-live="polite">
        {toasts.map((toast) => (
          <Toast
            key={toast.id}
            message={toast.message}
            tone={toast.tone}
            onClose={() => dismiss(toast.id)}
          />
        ))}
      </section>
    </ToastContext.Provider>
  );
}
