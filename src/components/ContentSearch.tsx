// Piezas del buscador que junta Modrinth y CurseForge.
import { Info } from "lucide-react";
import { useT } from "../i18n";
import type { SearchSource } from "../lib/types";
import type { ContentSearch } from "../lib/useContentSearch";

export const sourceName = (source: SearchSource) => (source === "modrinth" ? "Modrinth" : "CurseForge");

/** De dónde viene un resultado (el punto de color nunca va solo: lleva el nombre). */
export function SourceTag({ source }: { source: SearchSource }) {
  return (
    <span className="tag source-tag" data-source={source}>
      {sourceName(source)}
    </span>
  );
}

/** Avisos de la búsqueda: sin clave de CurseForge o una fuente que no respondió. */
export function SearchNotes({ search }: { search: ContentSearch }) {
  const t = useT();
  if (search.hits === null) return null;
  const notes = [
    ...(search.curseforgeEnabled ? [] : [t.search.onlyModrinth]),
    ...search.failed.map((f) => t.search.sourceFailed(sourceName(f.source), f.error)),
  ];
  if (notes.length === 0) return null;
  return (
    <>
      {notes.map((note) => (
        <div key={note} className="notice">
          <Info size={16} />
          <span>{note}</span>
        </div>
      ))}
    </>
  );
}

/** «Ver más» mientras alguna fuente tenga más resultados (y el error si la página no llegó). */
export function MoreButton({ search }: { search: ContentSearch }) {
  const t = useT();
  if (!search.hasMore) return null;
  return (
    <div className="mod-results-more">
      <button className="btn btn-ghost btn-sm" onClick={search.more} disabled={search.loadingMore}>
        {search.loadingMore ? t.common.loading : search.error ? t.common.retry : t.search.more}
      </button>
      {search.error && <span className="error-text">{t.search.failed(search.error)}</span>}
    </div>
  );
}
