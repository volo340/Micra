import { useEffect, useState } from "react";
import { Info, RefreshCw } from "lucide-react";
import { api, errorMessage } from "../../lib/api";
import type { MinecraftVersion } from "../../lib/types";

interface Props {
  fabric: boolean;
  value: string | null;
  onChange: (version: string) => void;
}

export function VersionStep({ fabric, value, onChange }: Props) {
  const [versions, setVersions] = useState<MinecraftVersion[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    setError(null);
    api
      .minecraftVersions()
      .then((list) => {
        setVersions(list);
        if (!value && list.length) onChange(list[0].id);
      })
      .catch((e) => setError(errorMessage(e)));
  };

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(load, []);

  const recent = versions?.slice(0, 6) ?? [];

  return (
    <div className="step">
      <div className="step-intro">
        <h2>Elige la versión de Minecraft</h2>
        <p className="muted">Tus amigos tendrán que abrir Minecraft en esta misma versión para entrar.</p>
      </div>

      {error ? (
        <div className="notice" data-tone="error">
          <span>No se pudo cargar la lista de versiones: {error}</span>
          <button className="btn btn-secondary btn-sm" onClick={load}>
            <RefreshCw size={14} /> Reintentar
          </button>
        </div>
      ) : (
        <>
          <div className="field">
            <span className="field-label">Versiones recientes</span>
            <div className="version-chips">
              {versions === null
                ? Array.from({ length: 6 }, (_, i) => <span key={i} className="version-chip skeleton" />)
                : recent.map((v, i) => (
                    <button
                      key={v.id}
                      type="button"
                      className="version-chip"
                      data-selected={value === v.id || undefined}
                      onClick={() => onChange(v.id)}
                    >
                      {v.id}
                      {i === 0 && <span className="version-latest">Última</span>}
                    </button>
                  ))}
            </div>
          </div>

          <label className="field">
            <span className="field-label">Todas las versiones</span>
            <select
              className="select"
              value={value ?? ""}
              onChange={(e) => onChange(e.target.value)}
              disabled={versions === null}
            >
              {versions === null && <option>Cargando…</option>}
              {versions?.map((v) => (
                <option key={v.id} value={v.id}>
                  {v.id}
                </option>
              ))}
            </select>
          </label>
        </>
      )}

      {fabric && (
        <div className="notice">
          <Info size={16} />
          <span>Micra instalará la última versión estable de Fabric compatible con esta versión.</span>
        </div>
      )}
    </div>
  );
}
