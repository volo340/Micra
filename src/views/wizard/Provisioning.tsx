import { AlertTriangle, Check, CheckCircle2, LayoutDashboard, Power } from "lucide-react";
import { Banner } from "../../components/ui";
import { useT, type Dict } from "../../i18n";
import { formatBytes, formatPercent } from "../../lib/format";
import type { ProvisionStep, ServerConfig } from "../../lib/types";
import type { TaskProgress } from "../../lib/useTask";
import type { Kind } from "./draft";

/** El mod que se está instalando después de crear el servidor. */
export interface ModProgress {
  index: number;
  total: number;
  title: string;
}

export interface FailedMod {
  title: string;
  error: string;
}

interface Props {
  name: string;
  kind: Kind;
  /** Si el servidor sale de un modpack. */
  modpack: boolean;
  step: ProvisionStep | null;
  /** Progreso de la copia al importar. */
  copy: TaskProgress | null;
  javaMajor: number | null;
  /** Cuántos mods elegidos hay que instalar al final. */
  mods: number;
  modProgress: ModProgress | null;
  failedMods: FailedMod[];
  error: string | null;
  created: ServerConfig | null;
  onBack: () => void;
  onOpen: (start: boolean) => void;
}

function phaseOf(step: ProvisionStep | null): number {
  switch (step?.step) {
    case "installingJava":
    case "downloadingJava":
      return 1;
    case "installingLoader":
      return 2;
    case "finalizing":
      return 3;
    default:
      return 0;
  }
}

function detail(step: ProvisionStep | null, t: Dict): { text: string; ratio: number | null } | null {
  if (step?.step === "downloadingFiles") {
    return { text: t.wizard.fileOf(step.done, step.total), ratio: step.total ? step.done / step.total : null };
  }
  if (step?.step === "downloadingJava") {
    return step.total
      ? {
          text: t.wizard.bytesOf(formatBytes(step.downloaded), formatBytes(step.total)),
          ratio: step.downloaded / step.total,
        }
      : { text: formatBytes(step.downloaded), ratio: null };
  }
  return null;
}

export function Provisioning(props: Props) {
  const { name, kind, modpack, step, copy, javaMajor, mods, modProgress, failedMods, error, created, onBack, onOpen } =
    props;
  const t = useT();
  if (created) {
    return (
      <div className="card provision provision-done">
        <span className="provision-badge" data-tone="success">
          <CheckCircle2 size={30} />
        </span>
        <h2>{t.wizard.ready(created.name)}</h2>
        <p className="muted">{t.wizard.readyHint(created.mcVersion, created.javaMajor)}</p>
        {failedMods.length > 0 && (
          <div className="provision-warning">
            <Banner tone="warning">
              <strong>{t.wizard.modsFailed(failedMods.length)}</strong>
              <ul className="provision-failures">
                {failedMods.map((m) => (
                  <li key={m.title}>
                    {m.title}: <span className="selectable">{m.error}</span>
                  </li>
                ))}
              </ul>
            </Banner>
          </div>
        )}
        <div className="provision-actions">
          <button className="btn btn-secondary btn-lg" onClick={() => onOpen(false)}>
            <LayoutDashboard size={17} />
            {t.wizard.toPanel}
          </button>
          <button className="btn btn-primary btn-lg" onClick={() => onOpen(true)}>
            <Power size={17} />
            {t.wizard.startNow}
          </button>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="card provision provision-done">
        <span className="provision-badge" data-tone="error">
          <AlertTriangle size={30} />
        </span>
        <h2>{kind === "import" ? t.wizard.importFailed : t.wizard.createFailed}</h2>
        <p className="provision-error selectable">{error}</p>
        <div className="provision-actions">
          <button className="btn btn-secondary btn-lg" onClick={onBack}>
            {t.wizard.backToWizard}
          </button>
        </div>
      </div>
    );
  }

  let current: number;
  let info: { text: string; ratio: number | null } | null;
  let phases: string[];
  if (kind === "import") {
    const copying = !copy || copy.total === 0 || copy.done < copy.total;
    current = copying ? 0 : 1;
    const ratio = copy && copy.total ? copy.done / copy.total : null;
    info = copying && ratio !== null ? { text: formatPercent(ratio * 100), ratio } : null;
    phases = [t.wizard.phaseCopy, t.wizard.phasePrepare];
  } else {
    // Los mods elegidos se instalan cuando el servidor ya está creado.
    current = modProgress ? 4 : phaseOf(step);
    info = modProgress
      ? {
          text: t.wizard.modOf(modProgress.index + 1, modProgress.total, modProgress.title),
          ratio: modProgress.index / modProgress.total,
        }
      : detail(step, t);
    phases = [
      modpack ? t.wizard.phasePack : t.wizard.phaseMinecraft,
      javaMajor ? t.wizard.phaseJavaN(javaMajor) : t.wizard.phaseJava,
      kind === "vanilla" ? t.wizard.phaseServer : t.wizard.phaseLoader,
      t.wizard.phaseConfig,
      ...(mods > 0 ? [t.wizard.phaseMods] : []),
    ];
  }

  return (
    <div className="card provision">
      <h2>{kind === "import" ? t.wizard.importing(name) : t.wizard.creating(name)}</h2>
      <p className="muted">{t.wizard.creatingHint}</p>
      <ol className="phases">
        {phases.map((label, i) => {
          const state = i < current ? "done" : i === current ? "current" : "todo";
          return (
            <li key={label} className="phase" data-state={state}>
              <span className="phase-dot">
                {state === "done" ? (
                  <Check size={13} strokeWidth={3} />
                ) : state === "current" ? (
                  <span className="spinner" />
                ) : null}
              </span>
              <div className="phase-body">
                <span className="phase-label">{label}</span>
                {state === "current" && (
                  <>
                    <div className="progress" data-indeterminate={info?.ratio == null || undefined}>
                      <span style={{ width: `${Math.round((info?.ratio ?? 0) * 100)}%` }} />
                    </div>
                    {info && <span className="phase-detail">{info.text}</span>}
                  </>
                )}
              </div>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
