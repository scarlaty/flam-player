import type { StoryEntry } from "../api/story";

/** Histoire fictive pour les tests (seul le titre est obligatoire). */
export function makeEntry(over: Partial<StoryEntry> & { title: string }): StoryEntry {
  return {
    source_id: "s",
    entry_id: `s:${over.title}`,
    uuid: over.title,
    version: null,
    description: null,
    age: null,
    category: null,
    author: null,
    voice: null,
    license: null,
    thumb_url: null,
    download_url: "https://exemple.org/x.zip",
    size: null,
    updated_at: null,
    kind: "telmi_zip",
    ...over,
  };
}
