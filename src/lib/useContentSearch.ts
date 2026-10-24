import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "./api";
import type { ContentHit, ContentPage, SearchCursor, SearchSource } from "./types";

export interface ContentSearch {
  /** `null` mientras llega la primera página. */
  hits: ContentHit[] | null;
  error: string | null;
  /** Fuentes que no respondieron (se muestran las demás). */
  failed: { source: SearchSource; error: string }[];
  /** Sin clave de API solo se busca en Modrinth. */
  curseforgeEnabled: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  more: () => void;
  retry: () => void;
}

/** Une una página a las anteriores sin repetir: el mismo proyecto de la otra fuente tampoco. */
function merge(previous: ContentHit[], page: ContentHit[]): ContentHit[] {
  const ids = new Set(previous.map((h) => `${h.source}:${h.id}`));
  const titles = new Map(previous.map((h) => [h.dedupeKey, h.source]));
  const out = [...previous];
  for (const hit of page) {
    const seen = titles.get(hit.dedupeKey);
    if (ids.has(`${hit.source}:${hit.id}`) || (seen && seen !== hit.source)) continue;
    ids.add(`${hit.source}:${hit.id}`);
    if (!seen) titles.set(hit.dedupeKey, hit.source);
    out.push(hit);
  }
  return out;
}

/**
 * Búsqueda en Modrinth y CurseForge a la vez: espera a que se deje de escribir,
 * pide más páginas y junta los resultados. `fetchPage` debe ser estable
 * (`useCallback`): al cambiar, la búsqueda empieza de nuevo.
 */
export function useContentSearch(
  query: string,
  fetchPage: (query: string, cursor: SearchCursor | null) => Promise<ContentPage>,
): ContentSearch {
  const [hits, setHits] = useState<ContentHit[] | null>(null);
  const [next, setNext] = useState<SearchCursor | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [failed, setFailed] = useState<ContentSearch["failed"]>([]);
  const [curseforgeEnabled, setCurseforgeEnabled] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const request = useRef(0);

  const apply = (page: ContentPage) => {
    setNext(page.next);
    setFailed(page.failed);
    setCurseforgeEnabled(page.curseforgeEnabled);
  };

  useEffect(() => {
    const ticket = ++request.current;
    setHits(null);
    setNext(null);
    setLoadingMore(false);
    const timer = window.setTimeout(
      () => {
        setError(null);
        fetchPage(query, null)
          .then((page) => {
            if (ticket !== request.current) return;
            setHits(merge([], page.hits));
            apply(page);
          })
          .catch((e) => ticket === request.current && setError(errorMessage(e)));
      },
      query ? 350 : 0,
    );
    return () => window.clearTimeout(timer);
  }, [query, fetchPage, attempt]);

  const more = useCallback(() => {
    if (!next || loadingMore) return;
    const ticket = request.current;
    setLoadingMore(true);
    setError(null);
    fetchPage(query, next)
      .then((page) => {
        if (ticket !== request.current) return;
        setHits((h) => merge(h ?? [], page.hits));
        apply(page);
      })
      .catch((e) => ticket === request.current && setError(errorMessage(e)))
      .finally(() => ticket === request.current && setLoadingMore(false));
  }, [next, loadingMore, fetchPage, query]);

  const retry = useCallback(() => setAttempt((n) => n + 1), []);

  return { hits, error, failed, curseforgeEnabled, hasMore: next !== null, loadingMore, more, retry };
}
