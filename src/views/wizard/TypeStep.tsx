import { Anvil, Box, Check, Feather, FolderInput } from "lucide-react";
import { useT } from "../../i18n";
import type { Kind } from "./draft";

const OPTIONS: { kind: Kind; icon: typeof Box }[] = [
  { kind: "vanilla", icon: Box },
  { kind: "fabric", icon: Feather },
  { kind: "forge", icon: Anvil },
  { kind: "import", icon: FolderInput },
];

interface Props {
  value: Kind;
  onChange: (kind: Kind) => void;
}

export function TypeStep({ value, onChange }: Props) {
  const t = useT();
  return (
    <div className="step">
      <div className="step-intro">
        <h2>{t.wizard.typeTitle}</h2>
        <p className="muted">{t.wizard.typeHint}</p>
      </div>
      <div className="option-grid" role="radiogroup">
        {OPTIONS.map(({ kind, icon: Icon }) => {
          const selected = value === kind;
          const option = t.wizard.kinds[kind];
          return (
            <button
              key={kind}
              type="button"
              role="radio"
              aria-checked={selected}
              className="option-card"
              data-selected={selected || undefined}
              onClick={() => onChange(kind)}
            >
              <span className="option-icon">
                <Icon size={22} />
              </span>
              <span className="option-text">
                <span className="option-title">
                  {option.title}
                  {kind === "vanilla" && <span className="option-tag">{t.wizard.startHere}</span>}
                </span>
                <span className="option-desc">{option.text}</span>
              </span>
              <span className="option-check">{selected && <Check size={14} strokeWidth={3} />}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
