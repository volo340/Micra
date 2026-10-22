import { Box, Check, Feather, Flame, PackageSearch } from "lucide-react";
import type { Kind } from "./draft";

const OPTIONS: { kind: Kind; icon: typeof Box; title: string; text: string; tag?: string }[] = [
  {
    kind: "vanilla",
    icon: Box,
    title: "Vanilla",
    text: "Minecraft tal cual, sin mods. Lo más sencillo para jugar con amigos.",
    tag: "Para empezar",
  },
  {
    kind: "fabric",
    icon: Feather,
    title: "Fabric",
    text: "Ligero y rápido, listo para añadir mods de rendimiento o de calidad de vida.",
  },
  {
    kind: "modrinth",
    icon: PackageSearch,
    title: "Modpack de Modrinth",
    text: "Elige entre miles de modpacks y Micra los instala en modo servidor.",
  },
  {
    kind: "curseforge",
    icon: Flame,
    title: "Modpack de CurseForge",
    text: "Usa el paquete de servidor oficial que publica cada autor.",
  },
];

interface Props {
  value: Kind;
  curseforgeEnabled: boolean;
  onChange: (kind: Kind) => void;
}

export function TypeStep({ value, curseforgeEnabled, onChange }: Props) {
  return (
    <div className="step">
      <div className="step-intro">
        <h2>¿Qué tipo de servidor quieres?</h2>
        <p className="muted">Podrás crear todos los servidores que quieras; empieza por uno.</p>
      </div>
      <div className="option-grid" role="radiogroup">
        {OPTIONS.map(({ kind, icon: Icon, title, text, tag }) => {
          const disabled = kind === "curseforge" && !curseforgeEnabled;
          const selected = value === kind;
          return (
            <button
              key={kind}
              type="button"
              role="radio"
              aria-checked={selected}
              className="option-card"
              data-selected={selected || undefined}
              disabled={disabled}
              onClick={() => onChange(kind)}
            >
              <span className="option-icon">
                <Icon size={22} />
              </span>
              <span className="option-text">
                <span className="option-title">
                  {title}
                  {tag && <span className="option-tag">{tag}</span>}
                </span>
                <span className="option-desc">
                  {disabled ? "Necesita una clave de API de CurseForge (mira Ajustes)." : text}
                </span>
              </span>
              <span className="option-check">{selected && <Check size={14} strokeWidth={3} />}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
