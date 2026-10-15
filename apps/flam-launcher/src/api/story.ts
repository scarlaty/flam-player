/** Nature de l'archive d'une histoire (miroir de `StoryKind` dans `src-tauri/src/story.rs`). */
export type StoryKind = "telmi_zip" | "flam_plain" | "flam_pk";

/**
 * Histoire proposée par une source, catalogue ou dossier local (miroir de `StoryEntry`
 * dans `src-tauri/src/story.rs`).
 */
export interface StoryEntry {
  source_id: string;
  uuid: string;
  version: number | null;
  title: string;
  description: string;
  age: number | null;
  category: string;
  author: string;
  voice: string;
  license: string;
  /** URL affichable telle quelle (`data:image/png;base64,…` pour un dossier local). */
  thumb_url: string | null;
  /** URL pour un catalogue, chemin local absolu pour un dossier. */
  download_url: string;
  size: number | null;
  /** Date de mise à jour (ISO 8601 UTC pour un dossier), chaîne vide si inconnue. */
  updated_at: string;
  kind: StoryKind;
}
