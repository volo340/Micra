import { useEffect, useState } from "react";
import { Info, RefreshCw } from "lucide-react";
import { useT } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import type { MinecraftVersion } from "../../lib/types";

interface Props {
  fabric: boolean;
  value: string | null;
  onChange: (version: string) => void;
}

export function VersionStep({ fabric, value, onChange }: Props) {
  const t = useT();
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
        <h2>{t.wizard.versionTitle}</h2>
        <p className="muted">{t.wizard.versionHint}</p>
      </div>

      {error ? (
        <div className="notice" data-tone="error">
          <span>{t.wizard.versionsFailed(error)}</span>
          <button className="btn btn-secondary btn-sm" onClick={load}>
            <RefreshCw size={14} /> {t.common.retry}
          </button>
        </div>
      ) : (
        <>
          <div className="field">
            <span className="field-label">{t.wizard.recent}</span>
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
                      {i === 0 && <span className="version-latest">{t.wizard.latest}</span>}
                    </button>
                  ))}
            </div>
          </div>

          <label className="field">
            <span className="field-label">{t.wizard.allVersions}</span>
            <select
              className="select"
              value={value ?? ""}
              onChange={(e) => onChange(e.target.value)}
              disabled={versions === null}
            >
              {versions === null && <option>{t.common.loading}</option>}
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
          <span>{t.wizard.fabricNote}</span>
        </div>
      )}
    </div>
  );
}
