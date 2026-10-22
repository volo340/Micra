import { useEffect, useState } from "react";
import { Box, FolderOpen, Gamepad2, Globe2, HardDrive, Info, Puzzle } from "lucide-react";
import { Banner } from "../../components/ui";
import { useT } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { formatBytes, loaderLabel } from "../../lib/format";
import type { ImportAnalysis, MinecraftVersion } from "../../lib/types";
import type { Draft } from "./draft";

interface Props {
  draft: Draft;
  onAnalysis: (analysis: ImportAnalysis) => void;
  onVersion: (version: string) => void;
}

/** Se puede importar si hay un jar de servidor o un instalador que ejecutar. */
export const importable = (a: ImportAnalysis | null) => Boolean(a && (a.launch || a.hasInstaller));

export function ImportStep({ draft, onAnalysis, onVersion }: Props) {
  const t = useT();
  const [analyzing, setAnalyzing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [versions, setVersions] = useState<MinecraftVersion[] | null>(null);
  const a = draft.analysis;
  const needsVersion = Boolean(a && !a.mcVersion);

  useEffect(() => {
    if (!needsVersion || versions !== null) return;
    api
      .minecraftVersions()
      .then((list) => {
        setVersions(list);
        if (!draft.mcVersion && list.length) onVersion(list[0].id);
      })
      .catch(() => setVersions([]));
  }, [needsVersion, versions, draft.mcVersion, onVersion]);

  const pick = async () => {
    setError(null);
    try {
      const path = await api.pickFolder();
      if (!path) return;
      setAnalyzing(true);
      onAnalysis(await api.analyzeImport(path));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setAnalyzing(false);
    }
  };

  const rows = a
    ? [
        { icon: FolderOpen, label: t.wizard.folder, value: a.path, mono: true },
        { icon: Gamepad2, label: t.wizard.version, value: a.mcVersion ?? t.wizard.notDetected },
        { icon: Box, label: t.wizard.loader, value: loaderLabel(a.loader) },
        { icon: HardDrive, label: t.wizard.size, value: formatBytes(a.size) },
        { icon: Globe2, label: t.wizard.world, value: a.hasWorld ? t.common.yes : t.common.no },
        { icon: Puzzle, label: t.wizard.mods, value: String(a.modCount) },
      ]
    : [];

  return (
    <div className="step">
      <div className="step-intro">
        <h2>{t.wizard.importTitle}</h2>
        <p className="muted">{t.wizard.importHint}</p>
      </div>

      <div className="button-row">
        <button className={a ? "btn btn-secondary" : "btn btn-primary"} onClick={pick} disabled={analyzing}>
          <FolderOpen size={16} />
          {a ? t.wizard.importOther : t.wizard.importPick}
        </button>
        {analyzing && (
          <span className="muted inline-wait">
            <span className="spinner" />
            {t.wizard.importAnalyzing}
          </span>
        )}
      </div>

      {error && (
        <div className="notice" data-tone="error">
          <span>{error}</span>
        </div>
      )}

      {a && (
        <>
          <dl className="summary">
            {rows.map(({ icon: Icon, label, value, mono }) => (
              <div key={label} className="summary-row">
                <dt>
                  <Icon size={15} />
                  {label}
                </dt>
                <dd className={mono ? "mono selectable" : undefined}>{value}</dd>
              </div>
            ))}
          </dl>

          {!importable(a) && <Banner tone="critical">{t.wizard.importNoLaunch}</Banner>}
          {needsVersion && (
            <label className="field">
              <span className="field-label">{t.wizard.importVersion}</span>
              <select
                className="select"
                value={draft.mcVersion ?? ""}
                onChange={(e) => onVersion(e.target.value)}
                disabled={!versions?.length}
              >
                {versions === null && <option>{t.common.loading}</option>}
                {versions?.map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.id}
                  </option>
                ))}
              </select>
              <span className="field-hint">{t.wizard.importVersionHint}</span>
            </label>
          )}
          {!a.launch && a.hasInstaller && (
            <div className="notice">
              <Info size={16} />
              <span>{t.wizard.importInstaller}</span>
            </div>
          )}
          {!a.onlineMode && <Banner tone="critical">{t.wizard.importOffline}</Banner>}
          {a.onlineMode && !a.whitelist && <Banner tone="warning">{t.wizard.importNoWhitelist}</Banner>}
          {!a.hasWorld && (
            <div className="notice">
              <Info size={16} />
              <span>{t.wizard.importNoWorld}</span>
            </div>
          )}
          <div className="notice">
            <Info size={16} />
            <span>{t.wizard.importCopies}</span>
          </div>
        </>
      )}
    </div>
  );
}
