import {
  Cpu,
  ExternalLink,
  FolderInput,
  Gamepad2,
  Hash,
  MemoryStick,
  Package,
  Puzzle,
  Type,
  UserRound,
} from "lucide-react";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import { formatCores, formatGb, loaderLabel, loaderName } from "../../lib/format";
import { isModpack, modLoader, modsToInstall, type Draft } from "./draft";

interface Props {
  draft: Draft;
  onChange: (patch: Partial<Draft>) => void;
}

export function ReviewStep({ draft, onChange }: Props) {
  const t = useT();
  const analysis = draft.analysis;
  const version = analysis?.mcVersion ?? draft.mcVersion;
  const modpack = isModpack(draft) ? draft.pack : null;
  // Con los mods sueltos, el loader exacto (NeoForge o Forge) ya está elegido.
  const loader = modpack ? null : modLoader(draft);
  const mods = modsToInstall(draft);
  const rows = [
    { icon: Type, label: t.wizard.name, value: draft.name.trim() },
    { icon: Package, label: t.wizard.type, value: loader ? loaderName(loader) : t.wizard.kinds[draft.kind].title },
    ...(analysis ? [{ icon: FolderInput, label: t.wizard.folder, value: analysis.path }] : []),
    modpack
      ? { icon: Gamepad2, label: t.wizard.modpack, value: `${modpack.title} · ${modpack.versionLabel ?? ""}` }
      : {
          icon: Gamepad2,
          label: t.wizard.version,
          value: `${t.common.minecraft(version ?? "?")}${analysis && analysis.loader.kind !== "vanilla" ? ` · ${loaderLabel(analysis.loader)}` : ""}`,
        },
    ...(mods.length > 0 ? [{ icon: Puzzle, label: t.wizard.mods, value: mods.map((m) => m.title).join(", ") }] : []),
    ...(draft.owner.trim() ? [{ icon: UserRound, label: t.wizard.ownerShort, value: draft.owner.trim() }] : []),
    { icon: MemoryStick, label: t.wizard.memory, value: t.wizard.upTo(formatGb(draft.maxMb)) },
    {
      icon: Cpu,
      label: t.wizard.cpu,
      value: draft.allCores ? t.wizard.allCores : t.common.cores(formatCores(draft.cores)),
    },
    { icon: Hash, label: t.resources.port, value: String(draft.port) },
  ];

  return (
    <div className="step">
      <div className="step-intro">
        <h2>{draft.kind === "import" ? t.wizard.reviewImportTitle : t.wizard.reviewTitle}</h2>
        <p className="muted">{draft.kind === "import" ? t.wizard.reviewImportHint : t.wizard.reviewHint}</p>
      </div>

      <dl className="summary">
        {rows.map(({ icon: Icon, label, value }) => (
          <div key={label} className="summary-row">
            <dt>
              <Icon size={15} />
              {label}
            </dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>

      <div className="eula" data-checked={draft.eula || undefined}>
        <label className="check">
          <input type="checkbox" checked={draft.eula} onChange={(e) => onChange({ eula: e.target.checked })} />
          <span>
            <strong>{t.wizard.eula}</strong>
            <span className="muted">{t.wizard.eulaHint}</span>
          </span>
        </label>
        <button className="btn btn-ghost btn-sm" onClick={() => api.openUrl("https://aka.ms/MinecraftEULA")}>
          <ExternalLink size={14} />
          {t.wizard.readEula}
        </button>
      </div>
    </div>
  );
}
