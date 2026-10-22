import { useEffect, useState } from "react";
import { ArrowLeft, ArrowRight, Check, Sparkles, X } from "lucide-react";
import { api, errorMessage, onProvisionStep } from "../../lib/api";
import type { AppInfo, HardwareInfo, ProvisionStep, ServerConfig } from "../../lib/types";
import { defaultMemory, isModpack, maxMemory, newDraft, toRequest, validPort, type Draft, type Kind } from "./draft";
import { PackStep } from "./PackStep";
import { Provisioning } from "./Provisioning";
import { ResourcesStep } from "./ResourcesStep";
import { ReviewStep } from "./ReviewStep";
import { TypeStep } from "./TypeStep";
import { VersionStep } from "./VersionStep";

const STEPS = ["Tipo", "Contenido", "Recursos", "Confirmar"];

interface Props {
  hardware: HardwareInfo | null;
  appInfo: AppInfo | null;
  onCancel: () => void;
  onCreated: (config: ServerConfig, start: boolean) => void;
}

export function Wizard({ hardware, appInfo, onCancel, onCreated }: Props) {
  const [draft, setDraft] = useState<Draft>(() => newDraft(hardware));
  const [step, setStep] = useState(0);
  const [creating, setCreating] = useState(false);
  const [progress, setProgress] = useState<ProvisionStep | null>(null);
  const [javaMajor, setJavaMajor] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<ServerConfig | null>(null);

  // Si los datos del equipo llegan tarde, ajustar la memoria a lo que cabe.
  useEffect(() => {
    if (hardware) setDraft((d) => ({ ...d, maxMb: Math.min(d.maxMb, maxMemory(hardware)) }));
  }, [hardware]);

  const update = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));

  const chooseKind = (kind: Kind) =>
    setDraft((d) =>
      d.kind === kind
        ? d
        : {
            ...d,
            kind,
            pack: null,
            maxMb: defaultMemory(kind, hardware),
            name: d.nameTouched ? d.name : "Mi servidor",
          },
    );

  const canNext = [
    true,
    isModpack(draft.kind) ? Boolean(draft.pack?.versionId) : Boolean(draft.mcVersion),
    draft.name.trim().length > 0 && validPort(draft.port) && (draft.allCores || draft.cores.length > 0),
    draft.eula,
  ][step];

  const create = async () => {
    setCreating(true);
    setError(null);
    setProgress(null);
    const unlisten = await onProvisionStep((p) => {
      setProgress(p);
      if (p.step === "installingJava") setJavaMajor(p.major);
    });
    try {
      setCreated(await api.createServer(toRequest(draft)));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      unlisten();
    }
  };

  return (
    <div className="page wizard">
      <header className="page-header">
        <div className="page-heading">
          <h1>Nuevo servidor</h1>
          <p className="muted">Cuatro pasos y a jugar.</p>
        </div>
        {!creating && (
          <button className="btn btn-ghost" onClick={onCancel}>
            <X size={16} />
            Cancelar
          </button>
        )}
      </header>

      {creating ? (
        <Provisioning
          name={draft.name.trim()}
          kind={draft.kind}
          step={progress}
          javaMajor={javaMajor}
          error={error}
          created={created}
          onBack={() => setCreating(false)}
          onOpen={(start) => created && onCreated(created, start)}
        />
      ) : (
        <>
          <ol className="stepper">
            {STEPS.map((label, i) => {
              const state = i < step ? "done" : i === step ? "current" : "todo";
              return (
                <li key={label} data-state={state}>
                  <button type="button" disabled={i >= step} onClick={() => setStep(i)}>
                    <span className="stepper-dot">
                      {state === "done" ? <Check size={13} strokeWidth={3} /> : i + 1}
                    </span>
                    {label}
                  </button>
                </li>
              );
            })}
          </ol>

          <div className="card wizard-card">
            {step === 0 && (
              <TypeStep
                value={draft.kind}
                curseforgeEnabled={Boolean(appInfo?.curseforgeEnabled)}
                onChange={chooseKind}
              />
            )}
            {step === 1 &&
              (isModpack(draft.kind) ? (
                <PackStep
                  source={draft.kind === "modrinth" ? "modrinth" : "curseforge"}
                  value={draft.pack}
                  onChange={(pack) =>
                    setDraft((d) => ({
                      ...d,
                      pack,
                      name: d.nameTouched || !pack ? d.name : pack.title.slice(0, 48),
                    }))
                  }
                />
              ) : (
                <VersionStep
                  fabric={draft.kind === "fabric"}
                  value={draft.mcVersion}
                  onChange={(mcVersion) => update({ mcVersion })}
                />
              ))}
            {step === 2 && <ResourcesStep draft={draft} hardware={hardware} onChange={update} />}
            {step === 3 && <ReviewStep draft={draft} onChange={update} />}
          </div>

          <footer className="wizard-footer">
            <button className="btn btn-ghost" onClick={() => setStep((s) => s - 1)} disabled={step === 0}>
              <ArrowLeft size={16} />
              Atrás
            </button>
            {step < STEPS.length - 1 ? (
              <button className="btn btn-primary" onClick={() => setStep((s) => s + 1)} disabled={!canNext}>
                Siguiente
                <ArrowRight size={16} />
              </button>
            ) : (
              <button className="btn btn-primary" onClick={create} disabled={!canNext}>
                <Sparkles size={16} />
                Crear servidor
              </button>
            )}
          </footer>
        </>
      )}
    </div>
  );
}
