import { AlertTriangle, Check, CheckCircle2, LayoutDashboard, Power } from "lucide-react";
import { formatBytes } from "../../lib/format";
import type { ProvisionStep, ServerConfig } from "../../lib/types";
import { isModpack, type Kind } from "./draft";

interface Props {
  name: string;
  kind: Kind;
  step: ProvisionStep | null;
  javaMajor: number | null;
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

function detail(step: ProvisionStep | null): { text: string; ratio: number | null } | null {
  if (step?.step === "downloadingFiles") {
    return { text: `Archivo ${step.done} de ${step.total}`, ratio: step.total ? step.done / step.total : null };
  }
  if (step?.step === "downloadingJava") {
    return step.total
      ? { text: `${formatBytes(step.downloaded)} de ${formatBytes(step.total)}`, ratio: step.downloaded / step.total }
      : { text: formatBytes(step.downloaded), ratio: null };
  }
  return null;
}

export function Provisioning({ name, kind, step, javaMajor, error, created, onBack, onOpen }: Props) {
  if (created) {
    return (
      <div className="card provision provision-done">
        <span className="provision-badge" data-tone="success">
          <CheckCircle2 size={30} />
        </span>
        <h2>¡«{created.name}» está listo!</h2>
        <p className="muted">
          Minecraft {created.mcVersion} con Java {created.javaMajor}. Enciéndelo y comparte la dirección con tus amigos.
        </p>
        <div className="provision-actions">
          <button className="btn btn-secondary btn-lg" onClick={() => onOpen(false)}>
            <LayoutDashboard size={17} />
            Ir al panel
          </button>
          <button className="btn btn-primary btn-lg" onClick={() => onOpen(true)}>
            <Power size={17} />
            Encender ahora
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
        <h2>No se pudo crear el servidor</h2>
        <p className="provision-error selectable">{error}</p>
        <div className="provision-actions">
          <button className="btn btn-secondary btn-lg" onClick={onBack}>
            Volver al asistente
          </button>
        </div>
      </div>
    );
  }

  const current = phaseOf(step);
  const info = detail(step);
  const phases = [
    isModpack(kind) ? "Descargar el modpack" : "Preparar Minecraft",
    javaMajor ? `Instalar Java ${javaMajor}` : "Instalar Java",
    kind === "vanilla" ? "Descargar el servidor" : "Instalar el servidor y el loader",
    "Configurar EULA, puerto y memoria",
  ];

  return (
    <div className="card provision">
      <h2>Creando «{name}»…</h2>
      <p className="muted">Puedes dejar Micra abierta y hacer otra cosa; te avisará al terminar.</p>
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
