import { useCallback, useEffect, useState } from "react";
import { ArrowLeft, ArrowRight, Check, Sparkles, X } from "lucide-react";
import { maxMemory, validPort } from "../../components/ResourceFields";
import { useT } from "../../i18n";
import { api, errorMessage, onProvisionStep } from "../../lib/api";
import { loaderName } from "../../lib/format";
import { validPlayerName } from "../../lib/serverSettings";
import type { AppInfo, HardwareInfo, ImportAnalysis, ProvisionStep, ServerConfig } from "../../lib/types";
import { useTask } from "../../lib/useTask";
import { ContentStep } from "./ContentStep";
import {
  defaultMemory,
  freePort,
  isHeavy,
  isModpack,
  modLoader,
  modrinthLoaders,
  modsToInstall,
  newDraft,
  toImportRequest,
  toRequest,
  usesLoader,
  type ContentMode,
  type Draft,
  type ForgeLoader,
  type Kind,
  type PackChoice,
  type PackSource,
} from "./draft";
import { ImportStep, importable } from "./ImportStep";
import { ModsStep } from "./ModsStep";
import { Provisioning, type FailedMod, type ModProgress } from "./Provisioning";
import { ResourcesStep } from "./ResourcesStep";
import { ReviewStep } from "./ReviewStep";
import { TypeStep } from "./TypeStep";
import { VersionStep } from "./VersionStep";

type StepId = "type" | "content" | "mods" | "resources" | "confirm";

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
  const [modProgress, setModProgress] = useState<ModProgress | null>(null);
  const [failedMods, setFailedMods] = useState<FailedMod[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<ServerConfig | null>(null);
  const copy = useTask(null);

  // Si los datos del equipo llegan tarde, ajustar la memoria a lo que cabe.
  useEffect(() => {
    if (hardware) setDraft((d) => ({ ...d, maxMb: Math.min(d.maxMb, maxMemory(hardware)) }));
  }, [hardware]);

  const update = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));
  const importing = draft.kind === "import";
  const choosingMods = usesLoader(draft.kind) && draft.content === "mods";
  const ids: StepId[] = ["type", "content", ...(choosingMods ? (["mods"] as const) : []), "resources", "confirm"];
  const labels: Record<StepId, string> = {
    type: t.wizard.steps.type,
    content: importing
      ? t.wizard.steps.folder
      : draft.kind === "vanilla"
        ? t.wizard.steps.version
        : t.wizard.steps.content,
    mods: t.wizard.steps.mods,
    resources: t.wizard.steps.resources,
    confirm: t.wizard.steps.confirm,
  };
  const current = ids[step];

  // Sin nombre propio, el servidor se llama como el modpack o como siempre.
  const defaultName = (d: Draft, pack: PackChoice | null) =>
    d.nameTouched ? d.name : pack ? pack.title.slice(0, 48) : t.wizard.defaultName;
  // Forge y los modpacks recomiendan más memoria que vanilla o Fabric.
  const withMemory = (d: Draft): Draft => ({ ...d, maxMb: defaultMemory(hardware, isHeavy(d)) });

  const chooseKind = (kind: Kind) =>
    setDraft((d) =>
      d.kind === kind
        ? d
        : withMemory({ ...d, kind, pack: null, mods: [], analysis: null, name: defaultName(d, null) }),
    );

  const chooseContent = (content: ContentMode) =>
    setDraft((d) =>
      d.content === content ? d : withMemory({ ...d, content, pack: null, name: defaultName(d, null) }),
    );

  // Los mods elegidos son de un loader y una versión: si cambian, se vacían.
  const chooseForgeLoader = (forgeLoader: ForgeLoader) =>
    setDraft((d) => (d.forgeLoader === forgeLoader ? d : { ...d, forgeLoader, mods: [] }));

  const onVersion = useCallback(
    (mcVersion: string) => setDraft((d) => (d.mcVersion === mcVersion ? d : { ...d, mcVersion, mods: [] })),
    [],
  );

  const choosePackSource = (packSource: PackSource) =>
    setDraft((d) => (d.packSource === packSource ? d : { ...d, packSource, pack: null, name: defaultName(d, null) }));

  const choosePack = (pack: PackChoice | null) => setDraft((d) => ({ ...d, pack, name: defaultName(d, pack) }));

  const onAnalysis = (a: ImportAnalysis) =>
    setDraft((d) => ({
      ...d,
      analysis: a,
      mcVersion: a.mcVersion ?? d.mcVersion,
      name: d.nameTouched ? d.name : a.suggestedName.slice(0, 48),
      port: a.port && !usedPorts.includes(a.port) && validPort(a.port) ? a.port : freePort(usedPorts),
      maxMb: defaultMemory(hardware, a.modCount > 0),
    }));

  const owner = draft.owner.trim();
  const ready: Record<StepId, boolean> = {
    type: true,
    content: importing
      ? importable(draft.analysis) && Boolean(draft.analysis?.mcVersion ?? draft.mcVersion)
      : isModpack(draft)
        ? Boolean(draft.pack?.versionId)
        : Boolean(draft.mcVersion),
    mods: true,
    resources:
      draft.name.trim().length > 0 &&
      validPort(draft.port) &&
      (draft.allCores || draft.cores.length > 0) &&
      (owner === "" || validPlayerName(owner)),
    confirm: draft.eula,
  };

  const create = async () => {
    setCreating(true);
    setError(null);
    setProgress(null);
    setModProgress(null);
    setFailedMods([]);
    const unlisten = await onProvisionStep((p) => {
      setProgress(p);
      if (p.step === "installingJava") setJavaMajor(p.major);
    });
    try {
      if (importing) {
        setCreated(await copy.run("import", () => api.importServer(toImportRequest(draft))));
        return;
      }
      const config = await api.createServer(toRequest(draft));
      // El servidor ya existe: un mod que falla no lo estropea, se avisa al final.
      const mods = modsToInstall(draft);
      const failed: FailedMod[] = [];
      for (const [index, mod] of mods.entries()) {
        setModProgress({ index, total: mods.length, title: mod.title });
        try {
          await api.installMod(config.id, mod.projectId);
        } catch (e) {
          failed.push({ title: mod.title, error: errorMessage(e) });
        }
      }
      setFailedMods(failed);
      setCreated(config);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      unlisten();
    }
  };

  const loader = modLoader(draft);

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
          modpack={isModpack(draft)}
          step={progress}
          copy={copy.progress}
          javaMajor={javaMajor}
          mods={modsToInstall(draft).length}
          modProgress={modProgress}
          failedMods={failedMods}
          error={error}
          created={created}
          onBack={() => setCreating(false)}
          onOpen={(start) => created && onCreated(created, start)}
        />
      ) : (
        <>
          <ol className="stepper">
            {ids.map((id, i) => {
              const state = i < step ? "done" : i === step ? "current" : "todo";
              return (
                <li key={id} data-state={state}>
                  <button type="button" disabled={i >= step} onClick={() => setStep(i)}>
                    <span className="stepper-dot">
                      {state === "done" ? <Check size={13} strokeWidth={3} /> : i + 1}
                    </span>
                    {labels[id]}
                  </button>
                </li>
              );
            })}
          </ol>

          <div className="card wizard-card">
            {current === "type" && <TypeStep value={draft.kind} onChange={chooseKind} />}
            {current === "content" &&
              (importing ? (
                <ImportStep draft={draft} onAnalysis={onAnalysis} onVersion={onVersion} />
              ) : usesLoader(draft.kind) ? (
                <ContentStep
                  draft={draft}
                  curseforgeEnabled={Boolean(appInfo?.curseforgeEnabled)}
                  onContent={chooseContent}
                  onForgeLoader={chooseForgeLoader}
                  onVersion={onVersion}
                  onPackSource={choosePackSource}
                  onPack={choosePack}
                />
              ) : (
                <VersionStep value={draft.mcVersion} onChange={onVersion} />
              ))}
            {current === "mods" && loader && draft.mcVersion && (
              <ModsStep
                loaders={modrinthLoaders(draft)}
                loaderName={loaderName(loader)}
                mcVersion={draft.mcVersion}
                value={draft.mods}
                onChange={(mods) => update({ mods })}
              />
            )}
            {current === "resources" && <ResourcesStep draft={draft} hardware={hardware} onChange={update} />}
            {current === "confirm" && <ReviewStep draft={draft} onChange={update} />}
          </div>

          <footer className="wizard-footer">
            <button className="btn btn-ghost" onClick={() => setStep((s) => s - 1)} disabled={step === 0}>
              <ArrowLeft size={16} />
              {t.common.back}
            </button>
            {step < ids.length - 1 ? (
              <button className="btn btn-primary" onClick={() => setStep((s) => s + 1)} disabled={!ready[current]}>
                {current === "mods" && draft.mods.length === 0 ? t.wizard.skipMods : t.common.next}
                <ArrowRight size={16} />
              </button>
            ) : (
              <button className="btn btn-primary" onClick={create} disabled={!ready[current]}>
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
