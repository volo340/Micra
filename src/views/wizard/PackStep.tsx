import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Download, PackageSearch, RefreshCw, Search } from "lucide-react";
import { MoreButton, SearchNotes, SourceTag } from "../../components/ContentSearch";
import { useT, type Dict } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { formatCompact, loaderName } from "../../lib/format";
import type { CfFile, ContentHit, SearchCursor, SearchSource } from "../../lib/types";
import { useContentSearch } from "../../lib/useContentSearch";
import type { PackChoice } from "./draft";

interface VersionOption {
  id: string;
  label: string;
}

/** CurseForge mezcla loaders ("Forge", "NeoForge") con versiones del juego. */
const LOADER_LABELS = ["forge", "neoforge", "fabric", "quilt"];
const fileLoaders = (f: CfFile) => f.gameVersions.map((v) => v.toLowerCase()).filter((v) => LOADER_LABELS.includes(v));

async function versionsOf(
  source: SearchSource,
  projectId: string,
  loaders: string[],
  t: Dict,
): Promise<VersionOption[]> {
  if (source === "modrinth") {
    const versions = await api.modrinthVersions(projectId, loaders);
    return versions.map((v) => ({
      id: v.id,
      label: `${v.version_number} · ${t.common.minecraft(v.game_versions.join(", "))} · ${v.loaders.map(loaderName).join(", ")}`,
    }));
  }
  // Solo sirven los archivos que tienen paquete de servidor (y son de este loader).
  const files = await api.curseForgeFiles(Number(projectId));
  return files
    .filter((f) => f.isServerPack || f.serverPackFileId !== null)
    .filter((f) => {
      const own = fileLoaders(f);
      return own.length === 0 || own.some((l) => loaders.includes(l));
    })
    .map((f) => ({
      id: String(f.id),
      label: `${f.displayName} · ${t.common.minecraft(f.gameVersions.filter((v) => /^\d/.test(v)).join(", "))}`,
    }));
}

export function PackIcon({ url, title, size = 48 }: { url: string | null; title: string; size?: number }) {
  const [failed, setFailed] = useState(false);
  return (
    <span className="pack-icon" style={{ width: size, height: size }}>
      {url && !failed ? (
        <img src={url} alt="" loading="lazy" onError={() => setFailed(true)} />
      ) : (
        <span>{title.slice(0, 1).toUpperCase()}</span>
      )}
    </span>
  );
}

interface Props {
  /** Solo modpacks de estos loaders (nombres de Modrinth). */
  loaders: string[];
  /** Nombre del tipo de servidor para los textos ("Fabric", "Forge / NeoForge"). */
  family: string;
  value: PackChoice | null;
  onChange: (pack: PackChoice | null) => void;
}

/** Buscador de modpacks de Modrinth y CurseForge con la versión elegida arriba. */
export function PackPicker({ loaders, family, value, onChange }: Props) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [versions, setVersions] = useState<VersionOption[] | null>(null);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const versionsRequest = useRef(0);
  const loaderKey = loaders.join(",");
  const fetchPage = useCallback(
    (q: string, cursor: SearchCursor | null) =>
      api.searchContent({ query: q, kind: "modpack", gameVersion: null, loaders: loaderKey.split(","), cursor }),
    [loaderKey],
  );
  const search = useContentSearch(query, fetchPage);
  const results = search.hits;

  const loadVersions = (keep: PackChoice) => {
    // Si se elige otro pack antes de que lleguen, estas ya no valen.
    const ticket = ++versionsRequest.current;
    setVersions(null);
    setVersionsError(null);
    versionsOf(keep.source, keep.projectId, loaders, t)
      .then((list) => {
        if (ticket !== versionsRequest.current) return;
        setVersions(list);
        // Al volver al paso se mantiene la versión que ya estaba elegida.
        const chosen = list.find((v) => v.id === keep.versionId) ?? list[0];
        onChange({ ...keep, versionId: chosen?.id ?? null, versionLabel: chosen?.label ?? null });
      })
      .catch((e) => ticket === versionsRequest.current && setVersionsError(errorMessage(e)));
  };

  // Al volver a este paso con un pack ya elegido, recargar sus versiones.
  useEffect(() => {
    if (value) loadVersions(value);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const select = (r: ContentHit) => {
    const choice: PackChoice = {
      source: r.source,
      projectId: r.id,
      title: r.title,
      iconUrl: r.iconUrl,
      versionId: null,
      versionLabel: null,
    };
    onChange(choice);
    loadVersions(choice);
  };

  return (
    <>
      {value && (
        <div className="pack-selection">
          <PackIcon url={value.iconUrl} title={value.title} size={44} />
          <div className="pack-selection-text">
            <span className="pack-selection-title">{value.title}</span>
            {versionsError ? (
              <span className="error-text">{t.wizard.packVersionsFailed(versionsError)}</span>
            ) : versions === null ? (
              <span className="subtle">{t.wizard.packVersionsLoading}</span>
            ) : versions.length === 0 ? (
              <span className="error-text">{t.wizard.packNoServer(family)}</span>
            ) : (
              <select
                className="select"
                value={value.versionId ?? ""}
                onChange={(e) => {
                  const option = versions.find((v) => v.id === e.target.value);
                  onChange({ ...value, versionId: e.target.value, versionLabel: option?.label ?? null });
                }}
                aria-label={t.wizard.packVersion}
              >
                {versions.map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.label}
                  </option>
                ))}
              </select>
            )}
          </div>
        </div>
      )}

      <div className="field">
        <div className="input-with-icon">
          <Search size={16} />
          <input
            className="input"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t.wizard.packSearch}
            aria-label={t.wizard.packSearch}
          />
        </div>
        <span className="field-hint">{t.wizard.packOnly(family)}</span>
      </div>

      <SearchNotes search={search} />

      {search.error && results === null ? (
        <div className="notice" data-tone="error">
          <span>{t.search.failed(search.error)}</span>
          <button className="btn btn-secondary btn-sm" onClick={search.retry}>
            <RefreshCw size={14} /> {t.common.retry}
          </button>
        </div>
      ) : results === null ? (
        <div className="pack-grid">
          {Array.from({ length: 6 }, (_, i) => (
            <div key={i} className="pack-card skeleton" />
          ))}
        </div>
      ) : results.length === 0 ? (
        <div className="empty">
          <span className="empty-icon">
            <PackageSearch size={20} />
          </span>
          <strong>{t.wizard.noResults}</strong>
          <span>{t.wizard.noResultsHint}</span>
        </div>
      ) : (
        <div className="pack-grid">
          {results.map((r) => {
            const selected = value?.source === r.source && value.projectId === r.id;
            return (
              <button
                key={`${r.source}:${r.id}`}
                type="button"
                className="pack-card"
                data-selected={selected || undefined}
                onClick={() => select(r)}
              >
                <PackIcon url={r.iconUrl} title={r.title} />
                <span className="pack-body">
                  <span className="pack-title">{r.title}</span>
                  {r.author && <span className="pack-author">{t.wizard.by(r.author)}</span>}
                  <span className="pack-desc">{r.description}</span>
                  <span className="pack-meta">
                    <SourceTag source={r.source} />
                    <span>
                      <Download size={12} />
                      {formatCompact(r.downloads)}
                    </span>
                    {r.gameVersion && <span>{t.common.minecraft(r.gameVersion)}</span>}
                  </span>
                </span>
                {selected && (
                  <span className="pack-check">
                    <Check size={13} strokeWidth={3} />
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
      {results && results.length > 0 && <MoreButton search={search} />}
    </>
  );
}
