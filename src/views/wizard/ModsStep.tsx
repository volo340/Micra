import { useCallback, useState } from "react";
import { Check, Download, PackageSearch, Plus, RefreshCw, Search, X } from "lucide-react";
import { MoreButton, SearchNotes, SourceTag } from "../../components/ContentSearch";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import { formatCompact } from "../../lib/format";
import type { ContentHit, SearchCursor } from "../../lib/types";
import { useContentSearch } from "../../lib/useContentSearch";
import { modKey, type ModChoice } from "./draft";
import { PackIcon } from "./PackStep";

interface Props {
  /** Loaders cuyos mods valen (nombres de Modrinth: "fabric", "neoforge"...). */
  loaders: string[];
  /** Nombre del loader para los textos. */
  loaderName: string;
  mcVersion: string;
  value: ModChoice[];
  onChange: (mods: ModChoice[]) => void;
}

/** Paso opcional: mods de Modrinth y CurseForge que se instalan al crear el servidor. */
export function ModsStep({ loaders, loaderName, mcVersion, value, onChange }: Props) {
  const t = useT();
  const [query, setQuery] = useState("");
  const loaderKey = loaders.join(",");
  const fetchPage = useCallback(
    (q: string, cursor: SearchCursor | null) =>
      api.searchContent({ query: q, kind: "mod", gameVersion: mcVersion, loaders: loaderKey.split(","), cursor }),
    [mcVersion, loaderKey],
  );
  const search = useContentSearch(query, fetchPage);
  const hits = search.hits;

  const chosen = new Set(value.map(modKey));
  const remove = (key: string) => onChange(value.filter((m) => modKey(m) !== key));
  const toggle = (h: ContentHit) => {
    const pick: ModChoice = { source: h.source, projectId: h.id, title: h.title, iconUrl: h.iconUrl };
    if (chosen.has(modKey(pick))) remove(modKey(pick));
    else onChange([...value, pick]);
  };

  return (
    <div className="step">
      <div className="step-intro">
        <h2>{t.wizard.modsTitle}</h2>
        <p className="muted">{t.wizard.modsHint(loaderName, mcVersion)}</p>
      </div>

      {value.length > 0 && (
        <div className="field">
          <span className="field-label">{t.wizard.modsChosen(value.length)}</span>
          <div className="mod-picks">
            {value.map((m) => (
              <span key={modKey(m)} className="mod-pick">
                <PackIcon url={m.iconUrl} title={m.title} size={22} />
                <span className="ellipsis">{m.title}</span>
                <button
                  type="button"
                  className="mod-pick-remove"
                  onClick={() => remove(modKey(m))}
                  aria-label={t.wizard.modRemove(m.title)}
                  title={t.wizard.modRemove(m.title)}
                >
                  <X size={13} />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}

      <div className="input-with-icon">
        <Search size={16} />
        <input
          className="input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t.wizard.modsSearch}
          aria-label={t.wizard.modsSearch}
        />
      </div>

      <SearchNotes search={search} />

      {search.error && hits === null ? (
        <div className="notice" data-tone="error">
          <span>{t.search.failed(search.error)}</span>
          <button className="btn btn-secondary btn-sm" onClick={search.retry}>
            <RefreshCw size={14} /> {t.common.retry}
          </button>
        </div>
      ) : hits === null ? (
        <div className="mod-results">
          {Array.from({ length: 5 }, (_, i) => (
            <div key={i} className="mod-result-skeleton skeleton" />
          ))}
        </div>
      ) : hits.length === 0 ? (
        <div className="empty">
          <span className="empty-icon">
            <PackageSearch size={20} />
          </span>
          <strong>{t.wizard.noResults}</strong>
          <span>{t.wizard.noResultsHint}</span>
        </div>
      ) : (
        <div className="mod-results">
          <ul className="rows">
            {hits.map((h) => {
              const key = modKey({ source: h.source, projectId: h.id });
              const selected = chosen.has(key);
              return (
                <li key={key} className="row" data-selected={selected || undefined}>
                  <PackIcon url={h.iconUrl} title={h.title} size={36} />
                  <span className="row-main">
                    <span className="row-title">
                      {h.title}
                      {h.author && <span className="subtle"> · {t.wizard.by(h.author)}</span>}
                    </span>
                    <span className="row-desc">{h.description}</span>
                  </span>
                  <span className="row-actions">
                    <SourceTag source={h.source} />
                    <span className="subtle downloads">
                      <Download size={12} />
                      {formatCompact(h.downloads)}
                    </span>
                    <button
                      className={selected ? "btn btn-primary btn-sm" : "btn btn-secondary btn-sm"}
                      aria-pressed={selected}
                      onClick={() => toggle(h)}
                    >
                      {selected ? <Check size={14} /> : <Plus size={14} />}
                      {selected ? t.wizard.modAdded : t.wizard.modAdd}
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
          <MoreButton search={search} />
        </div>
      )}
    </div>
  );
}
