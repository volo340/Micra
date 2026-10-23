import { Check, Info, Package, Puzzle } from "lucide-react";
import { Segmented } from "../../components/ui";
import { useT } from "../../i18n";
import { loaderName } from "../../lib/format";
import {
  modLoader,
  modrinthLoaders,
  type ContentMode,
  type Draft,
  type ForgeLoader,
  type PackChoice,
  type PackSource,
} from "./draft";
import { PackPicker } from "./PackStep";
import { VersionPicker } from "./VersionStep";

interface Props {
  draft: Draft;
  curseforgeEnabled: boolean;
  onContent: (content: ContentMode) => void;
  onForgeLoader: (loader: ForgeLoader) => void;
  /** Debe ser estable: la lista de versiones lo llama al cargar. */
  onVersion: (mcVersion: string) => void;
  onPackSource: (source: PackSource) => void;
  onPack: (pack: PackChoice | null) => void;
}

const MODES: { mode: ContentMode; icon: typeof Puzzle }[] = [
  { mode: "mods", icon: Puzzle },
  { mode: "modpack", icon: Package },
];

/** Fabric o Forge/NeoForge: versión y mods sueltos, o un modpack de ese loader. */
export function ContentStep({
  draft,
  curseforgeEnabled,
  onContent,
  onForgeLoader,
  onVersion,
  onPackSource,
  onPack,
}: Props) {
  const t = useT();
  const family = t.wizard.kinds[draft.kind].title;
  const loader = modLoader(draft);

  return (
    <div className="step">
      <div className="step-intro">
        <h2>{t.wizard.contentTitle(family)}</h2>
        <p className="muted">{t.wizard.contentHint}</p>
      </div>

      <div className="option-grid" role="radiogroup">
        {MODES.map(({ mode, icon: Icon }) => {
          const selected = draft.content === mode;
          const option = mode === "mods" ? t.wizard.contentMods : t.wizard.contentModpack;
          return (
            <button
              key={mode}
              type="button"
              role="radio"
              aria-checked={selected}
              className="option-card option-card-compact"
              data-selected={selected || undefined}
              onClick={() => onContent(mode)}
            >
              <span className="option-icon">
                <Icon size={20} />
              </span>
              <span className="option-text">
                <span className="option-title">{option.title}</span>
                <span className="option-desc">{option.text}</span>
              </span>
              <span className="option-check">{selected && <Check size={14} strokeWidth={3} />}</span>
            </button>
          );
        })}
      </div>

      {draft.content === "mods" ? (
        <>
          {draft.kind === "forge" && (
            <div className="field">
              <span className="field-label">{t.wizard.loader}</span>
              <Segmented
                value={draft.forgeLoader}
                options={[
                  { value: "neoForge", label: "NeoForge" },
                  { value: "forge", label: "Forge" },
                ]}
                onChange={onForgeLoader}
                label={t.wizard.loader}
              />
              <span className="field-hint">{t.wizard.forgeHint}</span>
            </div>
          )}
          <VersionPicker loader={loader} value={draft.mcVersion} onChange={onVersion} />
          {loader && (
            <div className="notice">
              <Info size={16} />
              <span>{t.wizard.loaderNote(loaderName(loader))}</span>
            </div>
          )}
        </>
      ) : (
        <>
          <div className="field">
            <span className="field-label">{t.wizard.packSource}</span>
            <Segmented
              value={draft.packSource}
              options={[
                { value: "modrinth", label: "Modrinth" },
                { value: "curseforge", label: "CurseForge", disabled: !curseforgeEnabled },
              ]}
              onChange={onPackSource}
              label={t.wizard.packSource}
            />
            {!curseforgeEnabled && <span className="field-hint">{t.wizard.curseforgeNeedsKey}</span>}
          </div>
          <PackPicker
            key={draft.packSource}
            source={draft.packSource}
            loaders={modrinthLoaders(draft)}
            family={family}
            value={draft.pack}
            onChange={onPack}
          />
        </>
      )}
    </div>
  );
}
