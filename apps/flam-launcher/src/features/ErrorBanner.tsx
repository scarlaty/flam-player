import type { ReactNode } from "react";
import type { AppError } from "../api/error";
import { Banner } from "../components";
import { t } from "../i18n";

/** Erreur du backend affichée en bandeau : titre selon le `kind`, puis le message. */
export function ErrorBanner({ error, action }: { error: AppError; action?: ReactNode }) {
  return (
    <Banner tone="danger" title={t.errors.title(error.kind)} action={action}>
      {/* Le message peut porter une seconde ligne (détail du backend sous un texte traduit). */}
      <p style={{ whiteSpace: "pre-line" }}>{error.message}</p>
    </Banner>
  );
}
