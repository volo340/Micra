import { useState } from "react";
import { AlertTriangle, CheckCircle2, FileText, Power, Puzzle, Trash2 } from "lucide-react";
import type { ServerTab } from "../../components/Sidebar";
import { useT, type Dict } from "../../i18n";
import { api } from "../../lib/api";
import type { Culprit, CulpritReason, ServerSummary } from "../../lib/types";

const nameOf = (c: Culprit) => c.name ?? c.fileName;

function reasonText(reason: CulpritReason, t: Dict): string {
  const r = t.crash.reasons;
  switch (reason.kind) {
    case "clientOnly":
      return r.clientOnly;
    case "missingDependency":
      return r.missingDependency(reason.dependency);
    case "wrongVersion":
      return r.wrongVersion(reason.needs);
    case "wrongLoader":
      return r.wrongLoader(reason.loader);
    case "incompatible":
      return r.incompatible(reason.other);
    case "duplicate":
      return r.duplicate;
    case "failed":
      return r.failed;
    case "suspected":
      return r.suspected;
  }
}

interface Props {
  server: ServerSummary;
  onTab: (tab: ServerTab) => void;
  onStart: () => void;
  onError: (e: unknown) => void;
}

/**
 * Por qué no arrancó (o se cayó) un servidor con mods: los mods que hay que
 * quitar, con un botón para quitarlos y volver a encenderlo.
 */
export function CrashCard({ server, onTab, onStart, onError }: Props) {
  const t = useT();
  const { config, status, live } = server;
  const id = config.id;
  const [working, setWorking] = useState<string | null>(null);
  const [removed, setRemoved] = useState<string[]>([]);
  const crash = live.crash;
  // Habla del último arranque: al volver a encenderlo deja de importar.
  if (!crash || (status !== "stopped" && status !== "crashed")) return null;
  if (!crash.culprits.length && !crash.missing.length && !crash.headline && !removed.length) return null;

  const locked = server.busy || working !== null;
  // Lo que señala el error, lo que solo aparece en él y lo que sobra en un servidor.
  const sure = crash.culprits.filter((c) => c.inLog && c.reason.kind !== "suspected");
  const suspected = crash.culprits.filter((c) => c.inLog && c.reason.kind === "suspected");
  const extra = crash.culprits.filter((c) => !c.inLog);
  const toRemove = [...sure, ...extra];

  const dismiss = () => api.dismissCrash(id).catch(onError);

  const remove = async (list: Culprit[], key: string) => {
    setWorking(key);
    try {
      for (const c of list) {
        await api.removeMod(id, c.fileName);
        setRemoved((r) => [...r, nameOf(c)]);
      }
      return true;
    } catch (e) {
      onError(e);
      return false;
    } finally {
      setWorking(null);
    }
  };

  const removeAndStart = async () => {
    if (await remove(toRemove, "all")) onStart();
  };

  if (!crash.culprits.length && removed.length) {
    return (
      <section className="crash-card" data-tone="success">
        <div className="crash-head">
          <span className="crash-icon">
            <CheckCircle2 size={18} />
          </span>
          <div className="crash-text">
            <h2>{t.crash.fixedTitle}</h2>
            <p>{t.crash.fixed(removed.join(", "))}</p>
          </div>
        </div>
        <div className="button-row">
          <button className="btn btn-primary btn-sm" onClick={onStart} disabled={server.busy}>
            <Power size={14} />
            {t.server.start}
          </button>
          <button className="btn btn-ghost btn-sm" onClick={dismiss}>
            {t.common.hide}
          </button>
        </div>
      </section>
    );
  }

  const intro = sure.length
    ? t.crash.intro
    : suspected.length
      ? t.crash.introSuspected
      : extra.length
        ? t.crash.introExtra
        : t.crash.introUnknown;
  const groups = [
    { key: "sure", title: null, items: sure },
    { key: "suspected", title: sure.length ? t.crash.suspected : null, items: suspected },
    { key: "extra", title: sure.length || suspected.length ? t.crash.extra : null, items: extra },
  ].filter((g) => g.items.length > 0);

  return (
    <section className="crash-card" role="alert">
      <div className="crash-head">
        <span className="crash-icon">
          <AlertTriangle size={18} />
        </span>
        <div className="crash-text">
          <h2>{crash.duringStart ? t.crash.titleStart : t.crash.titleStopped}</h2>
          <p>{intro}</p>
        </div>
      </div>

      {groups.map((g) => (
        <div key={g.key} className="crash-group">
          {g.title && <span className="crash-group-title">{g.title}</span>}
          <ul className="rows rows-flush">
            {g.items.map((c) => (
              <li key={c.fileName} className="row">
                <span className="mod-icon">{nameOf(c).slice(0, 1).toUpperCase()}</span>
                <span className="row-main">
                  <span className="row-title" title={c.fileName}>
                    {nameOf(c)}
                  </span>
                  <span className="row-desc">{reasonText(c.reason, t)}</span>
                </span>
                <span className="row-actions">
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() => remove([c], c.fileName)}
                    disabled={locked}
                  >
                    <Trash2 size={14} />
                    {working === c.fileName ? t.crash.removing : t.common.remove}
                  </button>
                </span>
              </li>
            ))}
          </ul>
        </div>
      ))}

      {crash.missing.length > 0 && (
        <div className="notice crash-missing">
          <Puzzle size={16} />
          <span>{t.crash.missing(crash.missing.length, crash.missing.join(", "))}</span>
          <button className="btn btn-secondary btn-sm" onClick={() => onTab("mods")}>
            {t.crash.toMods}
          </button>
        </div>
      )}

      {crash.headline && (
        <details className="crash-details">
          <summary>{t.crash.details}</summary>
          <code>{crash.headline}</code>
        </details>
      )}

      <div className="button-row">
        {toRemove.length > 0 && (
          <button className="btn btn-primary btn-sm" onClick={removeAndStart} disabled={locked}>
            <Trash2 size={14} />
            {working === "all" ? t.crash.removing : t.crash.removeAndStart(toRemove.length)}
          </button>
        )}
        {crash.reportFile && (
          <button className="btn btn-secondary btn-sm" onClick={() => api.openCrashReport(id).catch(onError)}>
            <FileText size={14} />
            {t.crash.openReport}
          </button>
        )}
        <button className="btn btn-ghost btn-sm" onClick={dismiss}>
          {t.common.hide}
        </button>
      </div>
    </section>
  );
}
