import { useState } from "react";
import { AlertTriangle, CheckCircle2, FileText, Power, Puzzle, Search, SlidersHorizontal, Trash2 } from "lucide-react";
import { SourceTag } from "../../components/ContentSearch";
import type { ServerTab } from "../../components/Sidebar";
import { useT, type Dict } from "../../i18n";
import { api } from "../../lib/api";
import { requestModSearch } from "../../lib/modSearchIntent";
import type { Confidence, CrashCause, Culprit, CulpritReason, MissingMod, ServerSummary } from "../../lib/types";

const nameOf = (c: { name: string | null; fileName: string }) => c.name ?? c.fileName;
const missingName = (m: MissingMod) => m.found?.title ?? m.id;
/** Se arregla con un clic: está desactivado o se sabe de dónde descargarlo. */
const fixable = (m: MissingMod) => m.disabledFile !== null || m.found !== null;

function reasonText(reason: CulpritReason, t: Dict): string {
  const r = t.crash.reasons;
  switch (reason.kind) {
    case "clientOnly":
      return r.clientOnly;
    case "wrongLoader":
      return r.wrongLoader(reason.loader);
    case "gameVersion":
      return r.gameVersion(reason.needs);
    case "loaderVersion":
      return r.loaderVersion(reason.loader, reason.needs);
    case "javaVersion":
      return r.javaVersion(reason.needs);
    case "dependencyVersion":
      return r.dependencyVersion(reason.dependency, reason.needs);
    case "missingDependency":
      return r.missingDependency(reason.dependency);
    case "incompatible":
      return r.incompatible(reason.other);
    case "duplicate":
      return r.duplicate;
    case "mixinFailed":
      return r.mixinFailed;
    case "failed":
      return r.failed;
    case "errorInCode":
      return r.errorInCode;
    case "suspected":
      return r.suspected;
  }
}

function causeText(cause: CrashCause, port: number, t: Dict): string {
  switch (cause.kind) {
    case "outOfMemory":
      return t.crash.causes.outOfMemory;
    case "portInUse":
      return t.crash.causes.portInUse(port);
    case "watchdog":
      return t.crash.causes.watchdog;
  }
}

const GROUPS: Confidence[] = ["certain", "likely", "possible"];

interface Props {
  server: ServerSummary;
  onTab: (tab: ServerTab) => void;
  onStart: () => void;
  onError: (e: unknown) => void;
}

/**
 * Por qué no arrancó (o se cayó) un servidor con mods y cómo arreglarlo: los
 * mods que faltan (se instalan) y los que hay que quitar. Solo se marcan
 * para quitar los que el error señala con seguridad; los dudosos se
 * muestran sin marcar.
 */
export function CrashCard({ server, onTab, onStart, onError }: Props) {
  const t = useT();
  const { config, status, live } = server;
  const id = config.id;
  const [working, setWorking] = useState<string | null>(null);
  /** Lo que la persona marcó o desmarcó a mano (por archivo). */
  const [picked, setPicked] = useState<Record<string, boolean>>({});
  const [removed, setRemoved] = useState<string[]>([]);
  const [installed, setInstalled] = useState<string[]>([]);
  const crash = live.crash;
  // Habla del último arranque: al volver a encenderlo deja de importar.
  if (!crash || (status !== "stopped" && status !== "crashed")) return null;
  const { culprits, missing } = crash;
  const done = removed.length + installed.length > 0;
  if (!culprits.length && !missing.length && !crash.cause && !crash.headline && !done) return null;

  const locked = server.busy || working !== null;
  // Lo seguro va marcado; lo probable, solo si no hay nada seguro ni falta nada.
  const certain = culprits.some((c) => c.confidence === "certain");
  const byDefault = (c: Culprit) =>
    c.confidence === "certain" || (c.confidence === "likely" && !certain && !missing.length);
  const isPicked = (c: Culprit) => picked[c.fileName] ?? byDefault(c);
  const toRemove = culprits.filter(isPicked);
  const toInstall = missing.filter(fixable);
  // Mientras se busca dónde descargar lo que falta, o si algo no se encontró,
  // encender ahora volvería a fallar.
  const searching = !crash.missingChecked && missing.some((m) => !fixable(m));
  const unresolved = crash.missingChecked && missing.some((m) => !fixable(m));

  const run = async (key: string, steps: (() => Promise<void>)[]) => {
    setWorking(key);
    try {
      for (const step of steps) await step();
      return true;
    } catch (e) {
      onError(e);
      return false;
    } finally {
      setWorking(null);
    }
  };
  const removeStep = (fileName: string, name: string) => async () => {
    await api.removeMod(id, fileName);
    setRemoved((r) => [...r, name]);
  };
  const installStep = (m: MissingMod) => async () => {
    if (m.disabledFile) await api.setModEnabled(id, m.disabledFile, true);
    else await api.installMissingMod(id, m.id);
    setInstalled((r) => [...r, missingName(m)]);
  };

  const fixAndStart = async () => {
    const steps = [...toInstall.map(installStep), ...toRemove.map((c) => removeStep(c.fileName, nameOf(c)))];
    if (await run("all", steps)) onStart();
  };
  const searchFor = (m: MissingMod) => {
    requestModSearch(id, m.id.replace(/[_-]+/g, " "));
    onTab("mods");
  };
  const dismiss = () => api.dismissCrash(id).catch(onError);

  if (!culprits.length && !missing.length && done) {
    return (
      <section className="crash-card" data-tone="success">
        <div className="crash-head">
          <span className="crash-icon">
            <CheckCircle2 size={18} />
          </span>
          <div className="crash-text">
            <h2>{t.crash.fixedTitle}</h2>
            <p>{t.crash.fixed(removed.join(", "), installed.join(", "))}</p>
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

  const sure = culprits.some((c) => c.confidence !== "possible") || missing.length > 0;
  const intro = crash.cause
    ? causeText(crash.cause, config.port, t)
    : sure
      ? t.crash.intro
      : culprits.length
        ? t.crash.introUnsure
        : t.crash.introUnknown;
  const groups = GROUPS.map((level) => ({
    level,
    items: culprits.filter((c) => c.confidence === level),
  })).filter((g) => g.items.length > 0);
  const fixes = toRemove.length + toInstall.length;

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

      {missing.length > 0 && (
        <div className="crash-group">
          <span className="crash-group-title">{t.crash.groups.missing}</span>
          <ul className="rows rows-flush">
            {missing.map((m) => {
              const requesters = m.requiredBy.map(nameOf).join(", ");
              const lost = crash.missingChecked && !fixable(m);
              return (
                <li key={m.id} className="row">
                  <span className="row-icon">
                    <Puzzle size={15} />
                  </span>
                  <span className="row-main">
                    <span className="row-title" title={m.id}>
                      {missingName(m)}
                      {m.version && <span className="subtle"> · {m.version}</span>}
                    </span>
                    {(requesters || lost) && (
                      <span className="row-desc">
                        {[requesters && t.crash.neededBy(requesters), lost && t.crash.notFound]
                          .filter(Boolean)
                          .join(". ")}
                      </span>
                    )}
                  </span>
                  <span className="row-actions">
                    {m.disabledFile ? (
                      <button
                        className="btn btn-secondary btn-sm"
                        onClick={() => run(m.id, [installStep(m)])}
                        disabled={locked}
                      >
                        {t.common.enable}
                      </button>
                    ) : m.found ? (
                      <>
                        <SourceTag source={m.found.source} />
                        <button
                          className="btn btn-secondary btn-sm"
                          onClick={() => run(m.id, [installStep(m)])}
                          disabled={locked}
                        >
                          {working === m.id ? t.crash.installing : t.crash.install}
                        </button>
                      </>
                    ) : !crash.missingChecked ? (
                      <span className="inline-wait">
                        <span className="spinner" />
                        {t.crash.searching}
                      </span>
                    ) : (
                      <>
                        {m.requiredBy.length > 0 && (
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() =>
                              run(
                                m.id,
                                m.requiredBy.map((r) => removeStep(r.fileName, nameOf(r))),
                              )
                            }
                            disabled={locked}
                          >
                            <Trash2 size={14} />
                            {t.crash.removeRequester(requesters)}
                          </button>
                        )}
                        <button className="btn btn-secondary btn-sm" onClick={() => searchFor(m)}>
                          <Search size={14} />
                          {t.crash.search}
                        </button>
                      </>
                    )}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {groups.map((g) => (
        <div key={g.level} className="crash-group">
          <span className="crash-group-title">{t.crash.groups[g.level]}</span>
          <ul className="rows rows-flush">
            {g.items.map((c) => (
              <li key={c.fileName} className="crash-item">
                <div className="row">
                  <label className="check crash-pick">
                    <input
                      type="checkbox"
                      checked={isPicked(c)}
                      disabled={locked}
                      onChange={(e) => setPicked((p) => ({ ...p, [c.fileName]: e.target.checked }))}
                    />
                    <span className="row-main">
                      <span className="row-title" title={c.fileName}>
                        {nameOf(c)}
                      </span>
                      <span className="row-desc">{reasonText(c.reason, t)}</span>
                    </span>
                  </label>
                  <span className="row-actions">
                    <button
                      className="btn btn-ghost btn-icon btn-sm"
                      onClick={() => run(c.fileName, [removeStep(c.fileName, nameOf(c))])}
                      disabled={locked}
                      title={t.common.remove}
                      aria-label={`${t.common.remove}: ${nameOf(c)}`}
                    >
                      <Trash2 size={14} />
                    </button>
                  </span>
                </div>
                {c.evidence && (
                  <details className="crash-why">
                    <summary>{t.crash.why}</summary>
                    <code>{c.evidence}</code>
                  </details>
                )}
              </li>
            ))}
          </ul>
        </div>
      ))}

      {crash.headline && (
        <details className="crash-details">
          <summary>{t.crash.details}</summary>
          <code>{crash.headline}</code>
        </details>
      )}

      <div className="button-row">
        {fixes > 0 && (
          <button className="btn btn-primary btn-sm" onClick={fixAndStart} disabled={locked || searching || unresolved}>
            <Power size={14} />
            {working === "all" ? t.crash.fixing : t.crash.fix(toRemove.length, toInstall.length)}
          </button>
        )}
        {(crash.cause?.kind === "outOfMemory" || crash.cause?.kind === "portInUse") && (
          <button className="btn btn-secondary btn-sm" onClick={() => onTab("settings")}>
            <SlidersHorizontal size={14} />
            {t.crash.toSettings}
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
