import { createContext, useContext } from "react";

export type ToastTone = "info" | "success" | "danger";

export interface ToastOptions {
  message: string;
  tone?: ToastTone;
  /** Durée d'affichage en millisecondes ; 0 = jusqu'à fermeture manuelle. */
  duration?: number;
}

export interface ToastApi {
  /** Affiche une notification et renvoie son identifiant. */
  show: (options: ToastOptions) => number;
  dismiss: (id: number) => void;
}

export const ToastContext = createContext<ToastApi | null>(null);

/** Accès aux notifications ; nécessite un `ToastProvider` parent. */
export function useToast(): ToastApi {
  const api = useContext(ToastContext);
  if (!api) {
    throw new Error("useToast doit être utilisé sous un ToastProvider");
  }
  return api;
}
