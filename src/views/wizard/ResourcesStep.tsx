import { Cpu, Hash, MemoryStick, Type } from "lucide-react";
import { formatGb } from "../../lib/format";
import type { HardwareInfo } from "../../lib/types";
import { isModpack, maxMemory, validPort, type Draft } from "./draft";

interface Props {
  draft: Draft;
  hardware: HardwareInfo | null;
  onChange: (patch: Partial<Draft>) => void;
}

const PRESETS = [2048, 4096, 6144, 8192, 12288, 16384];

export function ResourcesStep({ draft, hardware, onChange }: Props) {
  const min = 1024;
  const max = maxMemory(hardware);
  const fill = ((draft.maxMb - min) / (max - min)) * 100;
  const cpus = hardware?.logicalCpus ?? 4;
  const modpack = isModpack(draft.kind);

  const setCores = (cores: number[]) => onChange({ cores: [...new Set(cores)].sort((a, b) => a - b) });
  const toggleCore = (c: number) =>
    setCores(draft.cores.includes(c) ? draft.cores.filter((x) => x !== c) : [...draft.cores, c]);

  return (
    <div className="step">
      <div className="step-intro">
        <h2>Nombre y recursos</h2>
        <p className="muted">Puedes dejar los valores recomendados; los cambiarás más adelante si hace falta.</p>
      </div>

      <label className="field">
        <span className="field-label">
          <Type size={14} /> Nombre del servidor
        </span>
        <input
          className="input"
          value={draft.name}
          maxLength={48}
          onChange={(e) => onChange({ name: e.target.value, nameTouched: true })}
          placeholder="Mi servidor"
        />
        <span className="field-hint">Es el nombre que verán tus amigos en la lista de servidores.</span>
      </label>

      <div className="field">
        <div className="field-row">
          <span className="field-label">
            <MemoryStick size={14} /> Memoria RAM
          </span>
          <span className="ram-value">{formatGb(draft.maxMb)}</span>
        </div>
        <input
          type="range"
          className="slider"
          min={min}
          max={max}
          step={512}
          value={draft.maxMb}
          style={{ ["--fill" as string]: `${fill}%` }}
          onChange={(e) => onChange({ maxMb: Number(e.target.value) })}
          aria-label="Memoria máxima del servidor"
        />
        <div className="preset-row">
          {PRESETS.filter((p) => p <= max).map((p) => (
            <button
              key={p}
              type="button"
              className="preset"
              data-selected={draft.maxMb === p || undefined}
              onClick={() => onChange({ maxMb: p })}
            >
              {formatGb(p)}
            </button>
          ))}
        </div>
        <span className="field-hint">
          {modpack ? "Para modpacks se recomiendan entre 6 y 8 GB." : "Con 2–4 GB va sobrado para jugar entre amigos."}{" "}
          {hardware && `Tu equipo tiene ${formatGb(hardware.totalMemoryMb)}; Micra deja el resto para el sistema.`}
        </span>
      </div>

      <div className="field">
        <div className="field-row">
          <span className="field-label">
            <Cpu size={14} /> Núcleos del procesador
          </span>
          <label className="switch-label">
            <input
              type="checkbox"
              className="switch"
              checked={draft.allCores}
              onChange={(e) =>
                onChange({
                  allCores: e.target.checked,
                  cores: draft.cores.length
                    ? draft.cores
                    : Array.from({ length: Math.max(1, Math.floor(cpus / 2)) }, (_, i) => i),
                })
              }
            />
            Usar todos (recomendado)
          </label>
        </div>
        {!draft.allCores && (
          <>
            <div className="core-grid">
              {Array.from({ length: cpus }, (_, c) => (
                <button
                  key={c}
                  type="button"
                  className="core"
                  data-selected={draft.cores.includes(c) || undefined}
                  onClick={() => toggleCore(c)}
                  aria-pressed={draft.cores.includes(c)}
                >
                  {c}
                </button>
              ))}
            </div>
            <div className="preset-row">
              <button
                type="button"
                className="preset"
                onClick={() => setCores(Array.from({ length: Math.max(1, Math.floor(cpus / 2)) }, (_, i) => i))}
              >
                La mitad
              </button>
              <button
                type="button"
                className="preset"
                onClick={() => setCores(Array.from({ length: Math.max(1, cpus - 2) }, (_, i) => i))}
              >
                Todos menos 2
              </button>
            </div>
            <span className="field-hint">
              {draft.cores.length === 0
                ? "Selecciona al menos un núcleo."
                : `El servidor usará ${draft.cores.length} de ${cpus} hilos; el resto queda libre para tu PC.`}
            </span>
          </>
        )}
      </div>

      <label className="field">
        <span className="field-label">
          <Hash size={14} /> Puerto
        </span>
        <input
          className="input input-narrow mono"
          type="number"
          min={1024}
          max={65535}
          value={Number.isNaN(draft.port) ? "" : draft.port}
          onChange={(e) => onChange({ port: e.target.valueAsNumber })}
          aria-invalid={!validPort(draft.port)}
        />
        <span className="field-hint">
          {validPort(draft.port)
            ? "El de Minecraft es 25565. Cámbialo solo si vas a tener varios servidores encendidos a la vez."
            : "Usa un número entre 1024 y 65535."}
        </span>
      </label>
    </div>
  );
}
