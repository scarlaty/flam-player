import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  ConflictChoice,
  FolderCreated,
  FolderMode,
  FolderReport,
} from "./authoring";
import { invokeCommand, toAppError } from "./error";
import type { InstallTask } from "./install";

/*
 * Fenêtre de l'éditeur d'histoires (lot 7, miroir de `src-tauri/src/authoring/ops_commands.rs`).
 * Chaque fenêtre d'éditeur a son dossier, retrouvé par le backend d'après la fenêtre qui
 * appelle : la page ne transmet que des ids d'éléments (jamais de chemin de cible), sauf les
 * fichiers choisis dans le sélecteur natif (sources, lues et jamais modifiées).
 */

/** Préfixe des fenêtres de l'éditeur (`EDITOR_LABEL_PREFIX`). */
export const EDITOR_LABEL_PREFIX = "editor-";

/** Événement : création lancée depuis un éditeur (tâche à suivre, `TASK_EVENT`). */
export const EDITOR_TASK_EVENT = "authoring:task";

/** Fin d'une histoire ou d'un livre (miroir de `Fin`). */
export type FolderFin = "menu" | "suivante" | "fin";

/** Démarrage d'un livre (miroir de `Debut`). */
export type BookStart = "chapitre" | "menu";

/** Élément de l'arbre, complet (miroir de `Node`, `authoring/tree.rs`). */
export type EditorNode =
  | {
      kind: "menu";
      id: string;
      dir: string;
      title: string;
      mode: FolderMode;
      fin: FolderFin;
      annoncer: boolean;
      image: string | null;
      title_audio: string | null;
      children: EditorNode[];
    }
  | {
      kind: "story";
      id: string;
      file: string;
      title: string;
      image: string | null;
      title_audio: string | null;
      bytes: number;
    };

export type EditorMenu = Extract<EditorNode, { kind: "menu" }>;

/** État vu d'un élément (miroir de `Stamp`), rendu avec la cible d'une opération. */
export interface Stamp {
  dir: boolean;
  len: number;
  modifiedMs: number;
  modifiedNs: number;
}

/** Élément visé (miroir de `Target`). */
export interface Target {
  id: string;
  stamp: Stamp;
}

export interface JournalView {
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
}

/** Réglages réglés de `_histoire.json` (`null` : absents). */
export interface StoryFields {
  titre: string | null;
  sous_titre: string | null;
  description: string | null;
  age: number | null;
  debut: BookStart | null;
  mode: FolderMode | null;
  fin: FolderFin | null;
  annoncer: boolean | null;
}

/** Réglages réglés d'un sous-dossier (`null` : hérités du dossier parent). */
export interface FolderFields {
  mode: FolderMode | null;
  fin: FolderFin | null;
  annoncer: boolean | null;
}

/** Vue de l'éditeur (miroir de `EditorView`). */
export interface EditorView {
  report: Omit<FolderReport, "root"> & { root: EditorNode };
  folder_name: string;
  stamps: Record<string, Stamp>;
  story: StoryFields;
  folders: Record<string, FolderFields>;
  journal: JournalView;
}

/** Élément non copié (miroir de `Skipped`). */
export interface Skipped {
  path: string;
  reason: string;
}

/** Résultat d'une opération (miroir de `EditorUpdate`). */
export interface EditorUpdate {
  label: string;
  ids: string[];
  skipped: Skipped[];
  /** « Vider la corbeille » : éléments envoyés dans la Corbeille Windows. */
  trashed: number | null;
  view: EditorView;
}

/** Modification de réglages : absent = inchangé, `null` = effacé (hérité / par défaut). */
export interface FolderPatch {
  mode?: FolderMode | null;
  fin?: FolderFin | null;
  annoncer?: boolean | null;
}

export interface StoryPatch extends FolderPatch {
  titre?: string | null;
  sousTitre?: string | null;
  description?: string | null;
  age?: number | null;
  debut?: BookStart | null;
}

/** Vrai dans une fenêtre d'éditeur (libellé `editor-<n>`). */
export function isEditorWindow(): boolean {
  try {
    return getCurrentWindow().label.startsWith(EDITOR_LABEL_PREFIX);
  } catch {
    return false;
  }
}

/** `authoring_editor_open` : ouvre `path` dans une fenêtre d'éditeur (ou ramène la sienne). */
export function openEditor(path: string): Promise<{ already_open: boolean }> {
  return invokeCommand("authoring_editor_open", { path });
}

export function editorView(): Promise<EditorView> {
  return invokeCommand("authoring_editor_view");
}

export function importFiles(menu: string, sources: string[]): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_import", { menu, sources });
}

export function setImage(target: Target, source: string): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_set_image", { target, source });
}

export function setTitleAudio(target: Target, source: string): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_set_title_audio", { target, source });
}

export function keepComplement(target: Target): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_keep", { target });
}

export function reorder(menu: string, order: string[]): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_reorder", { menu, order });
}

export function moveItem(
  target: Target,
  dest: string,
  before: string | null = null,
): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_move", { target, dest, before });
}

export function renameItem(
  target: Target,
  title: string,
  renameFile: boolean,
): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_rename", { target, title, renameFile });
}

export function newMenu(parent: string, name: string): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_new_menu", { parent, name });
}

export function deleteItem(target: Target): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_delete", { target });
}

export function setStorySettings(patch: StoryPatch): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_story_settings", { patch });
}

export function setFolderSettings(
  menu: string,
  patch: FolderPatch,
): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_folder_settings", { menu, patch });
}

export function undo(): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_undo");
}

export function redo(): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_redo");
}

export function emptyTrash(): Promise<EditorUpdate> {
  return invokeCommand("authoring_editor_empty_trash");
}

export function revealFolder(): Promise<void> {
  return invokeCommand("authoring_editor_reveal");
}

export function editorThumbnail(id: string, size: number): Promise<string> {
  return invokeCommand("authoring_editor_thumbnail", { id, size });
}

export function createFromEditor(
  choice: ConflictChoice | null,
): Promise<{ created: FolderCreated; view: EditorView }> {
  return invokeCommand("authoring_editor_create", { choice });
}

/** Extensions acceptées (miroir de `AUDIO_EXTENSIONS` et `IMAGE_EXTENSIONS`, `tree.rs`). */
export const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"];
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "bmp", "webp"];

/** Sélecteur natif de plusieurs fichiers ; `[]` si l'utilisateur annule. */
export async function pickFiles(
  title: string,
  filterName: string,
  extensions: string[],
): Promise<string[]> {
  try {
    const picked = await open({
      directory: false,
      multiple: true,
      title,
      filters: [{ name: filterName, extensions }],
    });
    if (picked === null) return [];
    return Array.isArray(picked) ? picked : [picked];
  } catch (error) {
    throw toAppError(error);
  }
}

/** Sélecteur natif d'un fichier ; `null` si l'utilisateur annule. */
export async function pickOneFile(
  title: string,
  filterName: string,
  extensions: string[],
): Promise<string | null> {
  try {
    const picked = await open({
      directory: false,
      multiple: false,
      title,
      filters: [{ name: filterName, extensions }],
    });
    return typeof picked === "string" ? picked : null;
  } catch (error) {
    throw toAppError(error);
  }
}

/** Écoute les créations lancées depuis un éditeur (fenêtre principale). */
export async function onEditorTask(
  handler: (task: InstallTask) => void,
): Promise<() => void> {
  try {
    return await listen<InstallTask>(EDITOR_TASK_EVENT, (event) =>
      handler(event.payload),
    );
  } catch {
    return () => {};
  }
}
