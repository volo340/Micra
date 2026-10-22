import { useCallback, useEffect, useState } from "react";
import { ArrowLeft, ArrowRight, Check, Sparkles, X } from "lucide-react";
import { maxMemory, validPort } from "../../components/ResourceFields";
import { useT } from "../../i18n";
import { api, errorMessage, onProvisionStep } from "../../lib/api";
import { validPlayerName } from "../../lib/serverSettings";
import type { AppInfo, HardwareInfo, ImportAnalysis, ProvisionStep, ServerConfig } from "../../lib/types";
import { useTask } from "../../lib/useTask";
import {
  defaultMemory,
  freePort,
  isModpack,
  newDraft,
  toImportRequest,
  toRequest,
  type Draft,
  type Kind,
} from "./draft";
import { ImportStep, importable } from "./ImportStep";
import { PackStep } from "./PackStep";
import { Provisioning } from "./Provisioning";
import { ResourcesStep } from "./ResourcesStep";
import { ReviewStep } from "./ReviewStep";
import { TypeStep } from "./TypeStep";
import { VersionStep } from "./VersionStep";

interface Props {
  hardware: HardwareInfo | null;
  appInfo: AppInfo | null;
  /** Puertos de los servidores que ya existen. */
  usedPorts: number[];
  onCancel: () => void;
  onCreated: (config: ServerConfig, start: boolean) => void;
}

export function Wizard({ hardware, appInfo, usedPorts, onCancel, onCreated }: Props) {
  const t = useT();
  const [draft, setDraft] = useState<Draft>(() => newDraft(hardware, t.wizard.defaultName, freePort(usedPorts)));
  const [step, setStep] = useState(0);
  const [creating, setCreating] = useState(false);
  const [progress, setProgress] = useState<ProvisionStep | null>(null);
  const [javaMajor, setJavaMajor] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<ServerConfig | null>(null);
  const copy = useTask(null);

  // Si los datos del equipo llegan tarde, ajustar la memoria a lo que cabe.
  useEffect(() => {
    if (hardware) setDraft((d) => ({ ...d, maxMb: Math.min(d.maxMb, maxMemory(hardware)) }));
  }, [hardware]);

  const update = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));
  const importing = draft.kind === "import";
  const steps = [
    t.wizard.steps.type,
    importing ? t.wizard.steps.folder : t.wizard.steps.content,
    t.wizard.steps.resources,
    t.wizard.steps.confirm,
  ];

  const chooseKind = (kind: Kind) =>
    setDraft((d) =>
      d.kind === kind
        ? d
        : {
            ...d,
            kind,
            pack: null,
            analysis: null,
            maxMb: defaultMemory(kind, hardware),
            name: d.nameTouched ? d.name : t.wizard.defaultName,
          },
    );

  const onAnalysis = (a: ImportAnalysis) =>
    setDraft((d) => ({
      ...d,
      analysis: a,
      mcVersion: a.mcVersion ?? d.mcVersion,
      name: d.nameTouched ? d.name : a.suggestedName.slice(0, 48),
      port: a.port && !usedPorts.includes(a.port) && validPort(a.port) ? a.port : freePort(usedPorts),
      maxMb: defaultMemory("import", hardware, a.modCount > 0),
    }));

  const onVersion = useCallback((mcVersion: string) => setDraft((d) => ({ ...d, mcVersion })), []);

  const owner = draft.owner.trim();
  const content = importing
    ? importable(draft.analysis) && Boolean(draft.analysis?.mcVersion ?? draft.mcVersion)
    : isModpack(draft.kind)
      ? Boolean(draft.pack?.versionId)
      : Boolean(draft.mcVersion);
  const canNext = [
    true,
    content,
    draft.name.trim().length > 0 &&
      validPort(draft.port) &&
      (draft.allCores || draft.cores.length > 0) &&
      (owner === "" || validPlayerName(owner)),
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
      setCreated(
        importing
          ? await copy.run("import", () => api.importServer(toImportRequest(draft)))
          : await api.createServer(toRequest(draft)),
      );
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
          <h1>{t.wizard.title}</h1>
          <p className="muted">{t.wizard.subtitle}</p>
        </div>
        {!creating && (
          <button className="btn btn-ghost" onClick={onCancel}>
            <X size={16} />
            {t.common.cancel}
          </button>
        )}
      </header>

      {creating ? (
        <Provisioning
          name={draft.name.trim()}
          kind={draft.kind}
          step={progress}
          copy={copy.progress}
          javaMajor={javaMajor}
          error={error}
          created={created}
          onBack={() => setCreating(false)}
          onOpen={(start) => created && onCreated(created, start)}
        />
      ) : (
        <>
          <ol className="stepper">
            {steps.map((label, i) => {
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
              (importing ? (
                <ImportStep draft={draft} onAnalysis={onAnalysis} onVersion={onVersion} />
              ) : isModpack(draft.kind) ? (
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
              {t.common.back}
            </button>
            {step < steps.length - 1 ? (
              <button className="btn btn-primary" onClick={() => setStep((s) => s + 1)} disabled={!canNext}>
                {t.common.next}
                <ArrowRight size={16} />
              </button>
            ) : (
              <button className="btn btn-primary" onClick={create} disabled={!canNext}>
                <Sparkles size={16} />
                {importing ? t.wizard.import : t.wizard.create}
              </button>
            )}
          </footer>
        </>
      )}
    </div>
  );
}
