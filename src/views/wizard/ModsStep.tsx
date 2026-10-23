import { useEffect, useRef, useState } from "react";
import { Check, Download, PackageSearch, Plus, RefreshCw, Search, X } from "lucide-react";
import { useT } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { formatCompact } from "../../lib/format";
import type { ModrinthHit } from "../../lib/types";
import type { ModChoice } from "./draft";
import { PackIcon } from "./PackStep";

interface Props {
  /** Loaders de Modrinth cuyos mods valen ("fabric", "neoforge"...). */
  loaders: string[];
  /** Nombre del loader para los textos. */
  loaderName: string;
  mcVersion: string;
  value: ModChoice[];
  onChange: (mods: ModChoice[]) => void;
}

/** Paso opcional: mods de Modrinth que se instalan al crear el servidor. */
export function ModsStep({ loaders, loaderName, mcVersion, value, onChange }: Props) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [hits, setHits] = useState<ModrinthHit[] | null>(null);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  const loaderKey = loaders.join(",");

  useEffect(() => {
    const ticket = ++request.current;
    setHits(null);
    const timer = window.setTimeout(
      () => {
        setError(null);
        api
          .searchModrinthMods(query, mcVersion, loaderKey.split(","))
          .then((r) => {
            if (ticket !== request.current) return;
            setHits(r.hits);
            setTotal(r.total_hits);
          })
          .catch((e) => ticket === request.current && setError(errorMessage(e)));
      },
      query ? 350 : 0,
    );
    return () => window.clearTimeout(timer);
  }, [query, mcVersion, loaderKey, attempt]);

  const more = () => {
    const ticket = request.current;
    api
      .searchModrinthMods(query, mcVersion, loaders, hits?.length ?? 0)
      .then((r) => ticket === request.current && setHits((h) => [...(h ?? []), ...r.hits]))
      .catch((e) => ticket === request.current && setError(errorMessage(e)));
  };

  const chosen = new Set(value.map((m) => m.projectId));
  const remove = (projectId: string) => onChange(value.filter((m) => m.projectId !== projectId));
  const toggle = (h: ModrinthHit) =>
    chosen.has(h.project_id)
      ? remove(h.project_id)
      : onChange([...value, { projectId: h.project_id, title: h.title, iconUrl: h.icon_url }]);

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
              <span key={m.projectId} className="mod-pick">
                <PackIcon url={m.iconUrl} title={m.title} size={22} />
                <span className="ellipsis">{m.title}</span>
                <button
                  type="button"
                  className="mod-pick-remove"
                  onClick={() => remove(m.projectId)}
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

      {error ? (
        <div className="notice" data-tone="error">
          <span>{t.wizard.searchFailed(error)}</span>
          <button className="btn btn-secondary btn-sm" onClick={() => setAttempt((n) => n + 1)}>
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
              const selected = chosen.has(h.project_id);
              return (
                <li key={h.project_id} className="row" data-selected={selected || undefined}>
                  <PackIcon url={h.icon_url} title={h.title} size={36} />
                  <span className="row-main">
                    <span className="row-title">
                      {h.title}
                      <span className="subtle"> · {t.wizard.by(h.author)}</span>
                    </span>
                    <span className="row-desc">{h.description}</span>
                  </span>
                  <span className="row-actions">
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
          {hits.length < total && (
            <div className="mod-results-more">
              <button className="btn btn-ghost btn-sm" onClick={more}>
                {t.mods.more}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
