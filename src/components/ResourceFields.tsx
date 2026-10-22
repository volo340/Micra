// Campos de RAM, núcleos y puerto: los usan el asistente y los ajustes del servidor.
import type { ReactNode } from "react";
import { Cpu, Hash, MemoryStick } from "lucide-react";
import { useT } from "../i18n";
import { formatGb } from "../lib/format";
import type { HardwareInfo } from "../lib/types";

const PRESETS = [2048, 4096, 6144, 8192, 12288, 16384];

/** Máximo del control de RAM: lo recomendado para este equipo, en pasos de 512 MB. */
export function maxMemory(hardware: HardwareInfo | null): number {
  const recommended = hardware?.recommendedMaxMb ?? 8192;
  return Math.max(2048, Math.floor(recommended / 512) * 512);
}

export const validPort = (port: number) => Number.isInteger(port) && port >= 1024 && port <= 65535;

export function MemoryField(props: {
  value: number;
  hardware: HardwareInfo | null;
  hint?: ReactNode;
  onChange: (mb: number) => void;
}) {
  const t = useT();
  const min = 1024;
  const max = Math.max(maxMemory(props.hardware), props.value);
  const fill = ((props.value - min) / (max - min)) * 100;
  return (
    <div className="field">
      <div className="field-row">
        <span className="field-label">
          <MemoryStick size={14} /> {t.resources.memory}
        </span>
        <span className="ram-value">{formatGb(props.value)}</span>
      </div>
      <input
        type="range"
        className="slider"
        min={min}
        max={max}
        step={512}
        value={props.value}
        style={{ ["--fill" as string]: `${fill}%` }}
        onChange={(e) => props.onChange(Number(e.target.value))}
        aria-label={t.resources.memoryLabel}
      />
      <div className="preset-row">
        {PRESETS.filter((p) => p <= max).map((p) => (
          <button
            key={p}
            type="button"
            className="preset"
            data-selected={props.value === p || undefined}
            onClick={() => props.onChange(p)}
          >
            {formatGb(p)}
          </button>
        ))}
      </div>
      <span className="field-hint">
        {props.hint} {props.hardware && t.resources.memoryHint(formatGb(props.hardware.totalMemoryMb))}
      </span>
    </div>
  );
}

export function CoresField(props: {
  allCores: boolean;
  cores: number[];
  cpus: number;
  onChange: (allCores: boolean, cores: number[]) => void;
}) {
  const t = useT();
  const { allCores, cores, cpus } = props;
  const half = () => Array.from({ length: Math.max(1, Math.floor(cpus / 2)) }, (_, i) => i);
  const set = (list: number[]) =>
    props.onChange(
      false,
      [...new Set(list)].sort((a, b) => a - b),
    );
  const toggle = (c: number) => set(cores.includes(c) ? cores.filter((x) => x !== c) : [...cores, c]);

  return (
    <div className="field">
      <div className="field-row">
        <span className="field-label">
          <Cpu size={14} /> {t.resources.cores}
        </span>
        <label className="switch-label">
          <input
            type="checkbox"
            className="switch"
            checked={allCores}
            onChange={(e) => props.onChange(e.target.checked, cores.length ? cores : half())}
          />
          {t.resources.allCores}
        </label>
      </div>
      {!allCores && (
        <>
          <div className="core-grid">
            {Array.from({ length: cpus }, (_, c) => (
              <button
                key={c}
                type="button"
                className="core"
                data-selected={cores.includes(c) || undefined}
                onClick={() => toggle(c)}
                aria-pressed={cores.includes(c)}
              >
                {c}
              </button>
            ))}
          </div>
          <div className="preset-row">
            <button type="button" className="preset" onClick={() => set(half())}>
              {t.resources.half}
            </button>
            <button
              type="button"
              className="preset"
              onClick={() => set(Array.from({ length: Math.max(1, cpus - 2) }, (_, i) => i))}
            >
              {t.resources.allButTwo}
            </button>
          </div>
          <span className="field-hint">
            {cores.length === 0 ? t.resources.pickOne : t.resources.coresHint(cores.length, cpus)}
          </span>
        </>
      )}
    </div>
  );
}

export function PortField(props: { value: number; hint?: ReactNode; onChange: (port: number) => void }) {
  const t = useT();
  const ok = validPort(props.value);
  return (
    <label className="field">
      <span className="field-label">
        <Hash size={14} /> {t.resources.port}
      </span>
      <input
        className="input input-narrow mono"
        type="number"
        min={1024}
        max={65535}
        value={Number.isNaN(props.value) ? "" : props.value}
        onChange={(e) => props.onChange(e.target.valueAsNumber)}
        aria-invalid={!ok}
      />
      <span className="field-hint">{ok ? (props.hint ?? t.resources.portHint) : t.resources.portInvalid}</span>
    </label>
  );
}
