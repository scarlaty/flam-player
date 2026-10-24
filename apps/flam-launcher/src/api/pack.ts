import { invokeCommand } from "./error";

/**
 * Pack de podcasts (miroir de `src-tauri/src/podcast/pack.rs`, #362). Le front ne manipule
 * que des identifiants (jamais d'adresse : D11).
 */

/** Épisode désigné par sa source et son identifiant opaque. */
export interface EpisodeRef {
  source_id: string;
  episode_id: string;
}

export type PackNode =
  | ({ kind: "episode" } & EpisodeRef)
  | { kind: "category"; name: string; entries: PackNode[] };

export type CoverChoice =
  | { kind: "default" }
  | { kind: "feed"; source_id: string }
  | ({ kind: "episode" } & EpisodeRef);

export interface PackMetadataInput {
  /** Vide : titre du flux s'il n'y en a qu'un, sinon obligatoire (N1 b). */
  title: string;
  /** `null` : copyright du flux s'il n'y en a qu'un, sinon vide. */
  subtitle: string | null;
  age: number;
  /** `null` : liste des épisodes, calculée à la création. */
  description: string | null;
}

export interface PackComposition {
  metadata: PackMetadataInput;
  cover: CoverChoice;
  /** Menu racine. */
  menu: PackNode[];
  /**
   * Titres lus à voix haute (#462) : épisodes, catégories et titre du pack. Absent : `false`
   * (omis par le backend quand il est faux).
   */
  titles_audio?: boolean;
  /** Voix choisie (`tts_voices`) ; absente : voix mémorisée du poste, sinon voix par défaut. */
  titles_voice?: string | null;
}

export type ProblemCode =
  | "no_episode"
  | "too_many_episodes"
  | "too_deep"
  | "too_many_options"
  | "empty_category"
  | "invalid_category_name"
  | "duplicate_episode"
  | "invalid_id"
  | "title_required"
  | "text_too_long"
  | "episode_unavailable"
  | "episode_too_large"
  | "pack_too_large"
  | "cover_unavailable";

export interface Problem {
  code: ProblemCode;
  /** Message du backend, en français (jamais d'adresse). */
  message: string;
  /** Indices depuis le menu racine ; `[]` : menu racine. */
  path: number[] | null;
  episode: EpisodeRef | null;
  field: "title" | "subtitle" | "description" | "cover" | null;
}

export interface ResolvedMetadata {
  title: string;
  subtitle: string;
  age: number;
  description: string | null;
}

export interface ValidationReport {
  valid: boolean;
  problems: Problem[];
  unavailable: EpisodeRef[];
  episodes: number;
  feeds: number;
  estimated_bytes: number;
  unknown_size: number;
  metadata: ResolvedMetadata | null;
  cover: CoverChoice | null;
}

/** Brouillon lu avec sa validation (`DraftView`). */
export interface DraftView {
  composition: PackComposition | null;
  /** Dernière révision enregistrée : la prochaine sauvegarde doit la dépasser. */
  revision: number;
  notice: string | null;
  report: ValidationReport | null;
}

export interface SaveOutcome {
  saved: boolean;
  revision: number;
}

export function readPackDraft(): Promise<DraftView> {
  return invokeCommand<DraftView>("pack_draft_read");
}

export function savePackDraft(
  composition: PackComposition,
  revision: number,
): Promise<SaveOutcome> {
  return invokeCommand<SaveOutcome>("pack_draft_save", { composition, revision });
}

export function clearPackDraft(revision: number): Promise<void> {
  return invokeCommand<void>("pack_draft_clear", { revision });
}

export function validatePack(composition: PackComposition): Promise<ValidationReport> {
  return invokeCommand<ValidationReport>("pack_validate", { composition });
}

/** Composition conservée (miroir de `CompositionSummary`, `podcast/compositions.rs`, #461). */
export interface CompositionSummary {
  id: string;
  name: string;
  /** Secondes Unix. */
  created_at: number;
  /** Date de la dernière création lancée depuis cette composition (secondes Unix). */
  last_used_at: number;
  packs: number;
  episodes: number;
  feeds: number;
  /** Flux dont la source a été supprimée (composition gardée). */
  missing_feeds: number;
}

export interface CompositionsView {
  /** Dernière utilisée d'abord. */
  compositions: CompositionSummary[];
  notice: string | null;
  max: number;
}

export function listPackCompositions(): Promise<CompositionsView> {
  return invokeCommand<CompositionsView>("pack_compositions_list");
}

export function renamePackComposition(id: string, name: string): Promise<void> {
  return invokeCommand<void>("pack_composition_rename", { id, name });
}

export function deletePackComposition(id: string): Promise<void> {
  return invokeCommand<void>("pack_composition_delete", { id });
}

/** Copie la composition dans le brouillon (qui est remplacé) ; l'éditeur la relit ensuite. */
export function openPackComposition(id: string): Promise<SaveOutcome> {
  return invokeCommand<SaveOutcome>("pack_composition_open", { id });
}
