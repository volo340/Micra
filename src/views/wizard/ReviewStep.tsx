import { Cpu, ExternalLink, Gamepad2, Hash, MemoryStick, Package, Type } from "lucide-react";
import { api } from "../../lib/api";
import { formatCores, formatGb } from "../../lib/format";
import type { Draft } from "./draft";

const KIND_LABEL = {
  vanilla: "Vanilla",
  fabric: "Fabric",
  modrinth: "Modpack de Modrinth",
  curseforge: "Modpack de CurseForge",
};

interface Props {
  draft: Draft;
  onChange: (patch: Partial<Draft>) => void;
}

export function ReviewStep({ draft, onChange }: Props) {
  const rows = [
    { icon: Type, label: "Nombre", value: draft.name.trim() },
    { icon: Package, label: "Tipo", value: KIND_LABEL[draft.kind] },
    draft.pack
      ? { icon: Gamepad2, label: "Modpack", value: `${draft.pack.title} · ${draft.pack.versionLabel ?? ""}` }
      : { icon: Gamepad2, label: "Versión", value: `Minecraft ${draft.mcVersion}` },
    { icon: MemoryStick, label: "Memoria", value: `Hasta ${formatGb(draft.maxMb)}` },
    {
      icon: Cpu,
      label: "Procesador",
      value: draft.allCores ? "Todos los núcleos" : `Núcleos ${formatCores(draft.cores)}`,
    },
    { icon: Hash, label: "Puerto", value: String(draft.port) },
  ];

  return (
    <div className="step">
      <div className="step-intro">
        <h2>Revisa y crea tu servidor</h2>
        <p className="muted">
          Micra descargará Java y los archivos necesarios. Puede tardar unos minutos la primera vez.
        </p>
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
            <strong>Acepto el EULA de Minecraft</strong>
            <span className="muted">
              Es el acuerdo de licencia de Mojang que cualquier servidor necesita aceptar. Micra lo marca por ti en los
              archivos del servidor.
            </span>
          </span>
        </label>
        <button className="btn btn-ghost btn-sm" onClick={() => api.openUrl("https://aka.ms/MinecraftEULA")}>
          <ExternalLink size={14} />
          Leer el EULA
        </button>
      </div>
    </div>
  );
}
