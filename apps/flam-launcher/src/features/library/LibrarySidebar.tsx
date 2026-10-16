import { SidebarSourceList, type SidebarSource } from "../../components";
import { libraryHash, useRoute, useSourceId } from "../../router";
import { useLibrary } from "./LibraryContext";

/** Sources dans l'ordre de `sources.json`, « Mes histoires » en tête, avec leur état. */
export function LibrarySidebar() {
  const { sources, summaries } = useLibrary();
  const route = useRoute();
  const requested = useSourceId();

  if (!sources || sources.length === 0) return null;

  const items: SidebarSource[] = sources.map((source) => ({
    id: source.id,
    label: source.name,
    kind: source.type,
    count: summaries[source.id]?.count,
    status: summaries[source.id]?.status,
  }));
  const current = requested ?? sources[0]?.id;
  const selectedId = route === "library" ? current : undefined;

  return (
    <SidebarSourceList
      sources={items}
      selectedId={selectedId}
      onSelect={(id) => {
        window.location.hash = libraryHash(id);
      }}
    />
  );
}
