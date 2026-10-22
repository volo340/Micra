import { Type, UserRound } from "lucide-react";
import { CoresField, MemoryField, PortField } from "../../components/ResourceFields";
import { useT } from "../../i18n";
import { validPlayerName } from "../../lib/serverSettings";
import type { HardwareInfo } from "../../lib/types";
import { isModpack, type Draft } from "./draft";

interface Props {
  draft: Draft;
  hardware: HardwareInfo | null;
  onChange: (patch: Partial<Draft>) => void;
}

export function ResourcesStep({ draft, hardware, onChange }: Props) {
  const t = useT();
  const cpus = hardware?.logicalCpus ?? 4;
  const modded = isModpack(draft.kind) || (draft.analysis?.modCount ?? 0) > 0;
  const owner = draft.owner.trim();

  return (
    <div className="step">
      <div className="step-intro">
        <h2>{t.wizard.resourcesTitle}</h2>
        <p className="muted">{t.wizard.resourcesHint}</p>
      </div>

      <label className="field">
        <span className="field-label">
          <Type size={14} /> {t.wizard.name}
        </span>
        <input
          className="input"
          value={draft.name}
          maxLength={48}
          onChange={(e) => onChange({ name: e.target.value, nameTouched: true })}
          placeholder={t.wizard.defaultName}
        />
        <span className="field-hint">{t.wizard.nameHint}</span>
      </label>

      {draft.kind !== "import" && (
        <label className="field">
          <span className="field-label">
            <UserRound size={14} /> {t.wizard.owner}
          </span>
          <input
            className="input input-narrow"
            value={draft.owner}
            maxLength={16}
            onChange={(e) => onChange({ owner: e.target.value })}
            placeholder="Steve"
            spellCheck={false}
            autoComplete="off"
            aria-invalid={owner.length > 0 && !validPlayerName(owner)}
          />
          <span className="field-hint">
            {owner.length > 0 && !validPlayerName(owner) ? t.players.invalidName : t.wizard.ownerHint}
          </span>
        </label>
      )}

      <MemoryField
        value={draft.maxMb}
        hardware={hardware}
        hint={modded ? t.wizard.memoryModpack : t.wizard.memoryVanilla}
        onChange={(maxMb) => onChange({ maxMb })}
      />

      <CoresField
        allCores={draft.allCores}
        cores={draft.cores}
        cpus={cpus}
        onChange={(allCores, cores) => onChange({ allCores, cores })}
      />

      <PortField value={draft.port} onChange={(port) => onChange({ port })} />
    </div>
  );
}
