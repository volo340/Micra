import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { useT } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import type { ModLoader } from "../../lib/types";

interface PickerProps {
  /** Sin loader, todas las versiones de Mojang. */
  loader: ModLoader | null;
  value: string | null;
  onChange: (version: string) => void;
}

/** Versiones recientes en botones y el resto en una lista. */
export function VersionPicker({ loader, value, onChange }: PickerProps) {
  const t = useT();
  const [versions, setVersions] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let live = true;
    setVersions(null);
    setError(null);
    const load = loader ? api.loaderGameVersions(loader) : api.minecraftVersions().then((l) => l.map((v) => v.id));
    load.then((list) => live && setVersions(list)).catch((e) => live && setError(errorMessage(e)));
    return () => {
      live = false;
    };
  }, [loader, attempt]);

  // La versión elegida puede no existir con este loader: entonces, la más nueva.
  useEffect(() => {
    if (versions?.length && (!value || !versions.includes(value))) onChange(versions[0]);
  }, [versions, value, onChange]);

  if (error) {
    return (
      <div className="notice" data-tone="error">
        <span>{t.wizard.versionsFailed(error)}</span>
        <button className="btn btn-secondary btn-sm" onClick={() => setAttempt((n) => n + 1)}>
          <RefreshCw size={14} /> {t.common.retry}
        </button>
      </div>
    );
  }

  const recent = versions?.slice(0, 6) ?? [];
  return (
    <>
      <div className="field">
        <span className="field-label">{t.wizard.recent}</span>
        <div className="version-chips">
          {versions === null
            ? Array.from({ length: 6 }, (_, i) => <span key={i} className="version-chip skeleton" />)
            : recent.map((v, i) => (
                <button
                  key={v}
                  type="button"
                  className="version-chip"
                  data-selected={value === v || undefined}
                  onClick={() => onChange(v)}
                >
                  {v}
                  {i === 0 && <span className="version-latest">{t.wizard.latest}</span>}
                </button>
              ))}
        </div>
      </div>

      <label className="field">
        <span className="field-label">{t.wizard.allVersions}</span>
        <select
          className="select"
          value={versions?.includes(value ?? "") ? (value ?? "") : ""}
          onChange={(e) => onChange(e.target.value)}
          disabled={versions === null}
        >
          {versions === null && <option>{t.common.loading}</option>}
          {versions?.map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
      </label>
    </>
  );
}

interface Props {
  value: string | null;
  onChange: (version: string) => void;
}

/** Paso de versión de un servidor vanilla. */
export function VersionStep({ value, onChange }: Props) {
  const t = useT();
  return (
    <div className="step">
      <div className="step-intro">
        <h2>{t.wizard.versionTitle}</h2>
        <p className="muted">{t.wizard.versionHint}</p>
      </div>
      <VersionPicker loader={null} value={value} onChange={onChange} />
    </div>
  );
}
