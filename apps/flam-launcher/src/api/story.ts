/** Nature de l'archive d'une histoire (miroir de `StoryKind` dans `src-tauri/src/story.rs`). */
export type StoryKind = "telmi_zip" | "flam_plain" | "flam_pk";

/**
 * Histoire proposée par une source, catalogue ou dossier local (miroir de `StoryEntry`
 * dans `src-tauri/src/story.rs`).
 */
export interface StoryEntry {
  source_id: string;
  /**
   * Identifiant stable et unique dans une source : clé de grille de l'interface et clé de cache
   * des vignettes. Opaque. Catalogue : `"<source_id>:<uuid>"`.
   */
  entry_id: string;
  uuid: string;
  version: number | null;
  title: string;
  description: string | null;
  age: number | null;
  category: string | null;
  author: string | null;
  voice: string | null;
  license: string | null;
  /**
   * Référence de la vignette, PAS une URL affichable (la CSP bloque les images distantes) : à
   * résoudre via la commande de vignette de la source (`getCatalogThumbnail` pour un catalogue,
   * qui renvoie une URL `data:`). Un dossier local peut fournir directement une URL `data:`.
   */
  thumb_url: string | null;
  /** URL pour un catalogue, chemin local absolu pour un dossier. */
  download_url: string;
  size: number | null;
  /** Date ISO 8601 UTC. */
  updated_at: string | null;
  /**
   * Nombre de téléchargements publié par un catalogue (`download_count`), `null` si inconnu
   * (dossier, bibliothèque, catalogue sans la donnée). Sert au tri par popularité.
   */
  download_count: number | null;
  kind: StoryKind;
}
