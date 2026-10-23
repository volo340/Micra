import { useEffect, useRef, useState } from "react";
import { Check, Download, PackageSearch, RefreshCw, Search } from "lucide-react";
import { useT, type Dict } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { formatCompact, loaderName } from "../../lib/format";
import type { CfFile } from "../../lib/types";
import type { PackChoice, PackSource } from "./draft";

interface PackResult {
  id: string;
  title: string;
  author: string | null;
  description: string;
  iconUrl: string | null;
  downloads: number;
  gameVersion: string | null;
}

interface VersionOption {
  id: string;
  label: string;
}

async function search(source: PackSource, query: string, loaders: string[]): Promise<PackResult[]> {
  if (source === "modrinth") {
    const res = await api.searchModrinth(query, loaders);
    return res.hits.map((h) => ({
      id: h.project_id,
      title: h.title,
      author: h.author,
      description: h.description,
      iconUrl: h.icon_url,
      downloads: h.downloads,
      gameVersion: h.versions.at(-1) ?? null,
    }));
  }
  const res = await api.searchCurseForge(query, loaders);
  return res.data.map((m) => ({
    id: String(m.id),
    title: m.name,
    author: null,
    description: m.summary,
    iconUrl: m.logo?.thumbnailUrl ?? null,
    downloads: m.downloadCount,
    gameVersion: m.latestFiles[0]?.gameVersions.find((v) => /^\d/.test(v)) ?? null,
  }));
}

/** CurseForge mezcla loaders ("Forge", "NeoForge") con versiones del juego. */
const LOADER_LABELS = ["forge", "neoforge", "fabric", "quilt"];
const fileLoaders = (f: CfFile) => f.gameVersions.map((v) => v.toLowerCase()).filter((v) => LOADER_LABELS.includes(v));

async function versionsOf(source: PackSource, projectId: string, loaders: string[], t: Dict): Promise<VersionOption[]> {
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
  source: PackSource;
  /** Solo modpacks de estos loaders (nombres de Modrinth). */
  loaders: string[];
  /** Nombre del tipo de servidor para los textos ("Fabric", "Forge / NeoForge"). */
  family: string;
  value: PackChoice | null;
  onChange: (pack: PackChoice | null) => void;
}

/** Buscador de modpacks de Modrinth o CurseForge con la versión elegida arriba. */
export function PackPicker({ source, loaders, family, value, onChange }: Props) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [results, setResults] = useState<PackResult[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [versions, setVersions] = useState<VersionOption[] | null>(null);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const request = useRef(0);
  const loaderKey = loaders.join(",");

  useEffect(() => {
    const id = ++request.current;
    setResults(null);
    const timer = window.setTimeout(
      () => {
        setError(null);
        search(source, query, loaderKey.split(","))
          .then((r) => id === request.current && setResults(r))
          .catch((e) => id === request.current && setError(errorMessage(e)));
      },
      query ? 350 : 0,
    );
    return () => window.clearTimeout(timer);
  }, [source, query, attempt, loaderKey]);

  const loadVersions = (projectId: string, keep: PackChoice) => {
    setVersions(null);
    setVersionsError(null);
    versionsOf(source, projectId, loaders, t)
      .then((list) => {
        setVersions(list);
        // Al volver al paso se mantiene la versión que ya estaba elegida.
        const chosen = list.find((v) => v.id === keep.versionId) ?? list[0];
        onChange({ ...keep, versionId: chosen?.id ?? null, versionLabel: chosen?.label ?? null });
      })
      .catch((e) => setVersionsError(errorMessage(e)));
  };

  // Al volver a este paso con un pack ya elegido, recargar sus versiones.
  useEffect(() => {
    if (value && value.source === source) loadVersions(value.projectId, value);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const select = (r: PackResult) => {
    const choice: PackChoice = {
      source,
      projectId: r.id,
      title: r.title,
      iconUrl: r.iconUrl,
      versionId: null,
      versionLabel: null,
    };
    onChange(choice);
    loadVersions(r.id, choice);
  };

  const sourceName = source === "modrinth" ? "Modrinth" : "CurseForge";

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
              <span className="error-text">{t.wizard.packNoServer}</span>
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
            placeholder={t.wizard.packSearch(sourceName)}
            aria-label={t.wizard.packSearch(sourceName)}
          />
        </div>
        <span className="field-hint">{t.wizard.packOnly(family)}</span>
      </div>

      {error ? (
        <div className="notice" data-tone="error">
          <span>{t.wizard.searchFailed(error)}</span>
          <button className="btn btn-secondary btn-sm" onClick={() => setAttempt((n) => n + 1)}>
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
            const selected = value?.projectId === r.id;
            return (
              <button
                key={r.id}
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
    </>
  );
}
