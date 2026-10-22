import { useEffect, useMemo, useState } from "react";
import { Eye, EyeOff, FileCog, Undo2 } from "lucide-react";
import { useToast } from "../../components/Toasts";
import { CardTitle, CopyButton } from "../../components/ui";
import { useT, type Dict } from "../../i18n";
import { api } from "../../lib/api";
import type { PropertiesView } from "../../lib/types";

type Field =
  | { key: string; kind: "bool" }
  | { key: string; kind: "int"; min: number; max: number }
  | { key: string; kind: "choice"; options: string[] }
  | { key: string; kind: "text"; max: number }
  | { key: string; kind: "secret" };

type GroupId = "general" | "world" | "players" | "advanced";

const GROUPS: { id: GroupId; fields: Field[] }[] = [
  {
    id: "general",
    fields: [
      { key: "motd", kind: "text", max: 150 },
      { key: "max-players", kind: "int", min: 1, max: 1000 },
      { key: "difficulty", kind: "choice", options: ["peaceful", "easy", "normal", "hard"] },
      { key: "gamemode", kind: "choice", options: ["survival", "creative", "adventure", "spectator"] },
      { key: "hardcore", kind: "bool" },
      { key: "pvp", kind: "bool" },
    ],
  },
  {
    id: "world",
    fields: [
      { key: "level-name", kind: "text", max: 80 },
      { key: "level-seed", kind: "text", max: 100 },
      {
        key: "level-type",
        kind: "choice",
        options: ["minecraft:normal", "minecraft:flat", "minecraft:large_biomes", "minecraft:amplified"],
      },
      { key: "view-distance", kind: "int", min: 2, max: 32 },
      { key: "simulation-distance", kind: "int", min: 2, max: 32 },
      { key: "spawn-protection", kind: "int", min: 0, max: 10000 },
      { key: "allow-nether", kind: "bool" },
      { key: "generate-structures", kind: "bool" },
      { key: "spawn-monsters", kind: "bool" },
      { key: "spawn-animals", kind: "bool" },
      { key: "spawn-npcs", kind: "bool" },
    ],
  },
  {
    id: "players",
    fields: [
      { key: "online-mode", kind: "bool" },
      { key: "white-list", kind: "bool" },
      { key: "enforce-whitelist", kind: "bool" },
      { key: "enforce-secure-profile", kind: "bool" },
      { key: "prevent-proxy-connections", kind: "bool" },
      { key: "hide-online-players", kind: "bool" },
      { key: "player-idle-timeout", kind: "int", min: 0, max: 1440 },
      { key: "op-permission-level", kind: "int", min: 1, max: 4 },
    ],
  },
  {
    id: "advanced",
    fields: [
      { key: "allow-flight", kind: "bool" },
      { key: "enable-command-block", kind: "bool" },
      { key: "force-gamemode", kind: "bool" },
      { key: "enable-rcon", kind: "bool" },
      { key: "rcon.port", kind: "int", min: 1024, max: 65535 },
      { key: "rcon.password", kind: "secret" },
      { key: "enable-query", kind: "bool" },
      { key: "query.port", kind: "int", min: 1024, max: 65535 },
      { key: "enable-status", kind: "bool" },
      { key: "sync-chunk-writes", kind: "bool" },
      { key: "max-tick-time", kind: "int", min: -1, max: 2147483647 },
      { key: "network-compression-threshold", kind: "int", min: -1, max: 65535 },
      { key: "resource-pack", kind: "text", max: 500 },
      { key: "require-resource-pack", kind: "bool" },
    ],
  },
];

/** Lo que usa Minecraft si la clave no está en el archivo (aún no ha arrancado nunca). */
const DEFAULTS: Record<string, string> = {
  motd: "A Minecraft Server",
  "max-players": "20",
  difficulty: "easy",
  gamemode: "survival",
  hardcore: "false",
  pvp: "true",
  "level-name": "world",
  "level-seed": "",
  "level-type": "minecraft:normal",
  "view-distance": "10",
  "simulation-distance": "10",
  "spawn-protection": "16",
  "allow-nether": "true",
  "generate-structures": "true",
  "spawn-monsters": "true",
  "spawn-animals": "true",
  "spawn-npcs": "true",
  "online-mode": "true",
  "white-list": "false",
  "enforce-whitelist": "false",
  "enforce-secure-profile": "true",
  "prevent-proxy-connections": "false",
  "hide-online-players": "false",
  "player-idle-timeout": "0",
  "op-permission-level": "4",
  "allow-flight": "false",
  "enable-command-block": "false",
  "force-gamemode": "false",
  "enable-rcon": "false",
  "rcon.port": "25575",
  "rcon.password": "",
  "enable-query": "false",
  "query.port": "25565",
  "enable-status": "true",
  "sync-chunk-writes": "true",
  "max-tick-time": "60000",
  "network-compression-threshold": "256",
  "resource-pack": "",
  "require-resource-pack": "false",
};

const KNOWN = new Set(GROUPS.flatMap((g) => g.fields.map((f) => f.key)));
/** Se cambia en la sección Red (Micra lo sincroniza al arrancar). */
const READ_ONLY = new Set(["server-port"]);

function invalid(field: Field, value: string): boolean {
  if (field.kind === "int") {
    const n = Number(value.trim());
    return !Number.isInteger(n) || n < field.min || n > field.max;
  }
  if (field.kind === "text") return value.length > field.max || /[\r\n]/.test(value);
  return false;
}

function choiceLabel(key: string, value: string, t: Dict): string {
  const names = t.properties.choices as Record<string, Record<string, string>>;
  return names[key]?.[value.replace(/^minecraft:/, "")] ?? value;
}

/** Aviso bajo los campos que más importan para la seguridad. */
function warning(key: string, value: string, t: Dict): { tone: "critical" | "warning" | "info"; text: string } | null {
  if (key === "online-mode" && value === "false") return { tone: "critical", text: t.properties.onlineModeOff };
  if (key === "white-list" && value === "false") return { tone: "warning", text: t.properties.whitelistOff };
  if (key === "enable-rcon" && value === "true") return { tone: "info", text: t.properties.rconNote };
  if (key === "enable-query" && value === "true") return { tone: "info", text: t.properties.queryNote };
  return null;
}

interface Props {
  serverId: string;
  view: PropertiesView;
  running: boolean;
  onSaved: (view: PropertiesView) => void;
  onError: (e: unknown) => void;
}

/** Editor visual de server.properties, agrupado y con avisos de seguridad. */
export function PropertiesEditor({ serverId, view, running, onSaved, onError }: Props) {
  const t = useT();
  const toast = useToast();
  const base = useMemo(() => ({ ...DEFAULTS, ...view.values }), [view]);
  const [draft, setDraft] = useState<Record<string, string>>(base);
  const [saving, setSaving] = useState(false);
  const [showSecret, setShowSecret] = useState(false);
  const [group, setGroup] = useState<GroupId | "other">("general");

  useEffect(() => setDraft(base), [base]);

  const others = Object.keys(view.values)
    .filter((k) => !KNOWN.has(k))
    .sort();
  const changes = Object.fromEntries(Object.entries(draft).filter(([k, v]) => v !== base[k] && !READ_ONLY.has(k)));
  const fields = GROUPS.flatMap((g) => g.fields);
  const errors = fields.filter((f) => f.key in changes && invalid(f, changes[f.key]));
  const dirty = Object.keys(changes).length > 0;
  const set = (key: string, value: string) => setDraft((d) => ({ ...d, [key]: value }));

  const save = async () => {
    setSaving(true);
    try {
      const next = await api.saveProperties(serverId, changes);
      onSaved(next);
      toast({
        tone: running ? "info" : "success",
        title: t.properties.saved,
        message: running ? t.common.appliesOnRestart : undefined,
      });
    } catch (e) {
      onError(e);
    } finally {
      setSaving(false);
    }
  };

  const renderField = (field: Field) => {
    const value = draft[field.key] ?? "";
    const label = (t.properties.labels as Record<string, string>)[field.key] ?? field.key;
    const note = warning(field.key, value, t);
    const id = `prop-${field.key}`;
    let control;
    switch (field.kind) {
      case "bool":
        control = (
          <input
            id={id}
            type="checkbox"
            className="switch"
            checked={value === "true"}
            onChange={(e) => set(field.key, e.target.checked ? "true" : "false")}
          />
        );
        break;
      case "choice": {
        const options = field.options.includes(value) || !value ? field.options : [value, ...field.options];
        control = (
          <select id={id} className="select" value={value} onChange={(e) => set(field.key, e.target.value)}>
            {options.map((o) => (
              <option key={o} value={o}>
                {choiceLabel(field.key, o, t)}
              </option>
            ))}
          </select>
        );
        break;
      }
      case "int":
        control = (
          <input
            id={id}
            className="input mono"
            type="number"
            min={field.min}
            max={field.max}
            value={value}
            onChange={(e) => set(field.key, e.target.value)}
            aria-invalid={invalid(field, value)}
          />
        );
        break;
      case "secret":
        control = (
          <div className="secret-input">
            <input
              id={id}
              className="input mono"
              type={showSecret ? "text" : "password"}
              value={value}
              maxLength={128}
              autoComplete="off"
              onChange={(e) => set(field.key, e.target.value)}
            />
            <button
              type="button"
              className="btn btn-ghost btn-icon btn-sm"
              onClick={() => setShowSecret((s) => !s)}
              title={showSecret ? t.common.hide : t.common.show}
              aria-label={showSecret ? t.common.hide : t.common.show}
            >
              {showSecret ? <EyeOff size={15} /> : <Eye size={15} />}
            </button>
            {value && <CopyButton text={value} />}
          </div>
        );
        break;
      default:
        control = (
          <input
            id={id}
            className="input"
            value={value}
            maxLength={field.max}
            onChange={(e) => set(field.key, e.target.value)}
            aria-invalid={invalid(field, value)}
          />
        );
    }
    return (
      <div key={field.key} className="prop" data-kind={field.kind}>
        <label className="prop-label" htmlFor={id}>
          <span>{label}</span>
          <span className="prop-key mono">{field.key}</span>
        </label>
        <div className="prop-control">{control}</div>
        {field.kind === "secret" && <span className="field-hint prop-note">{t.properties.rconPasswordHint}</span>}
        {note && (
          <span className="prop-note" data-tone={note.tone}>
            {note.text}
          </span>
        )}
      </div>
    );
  };

  const groupIds: (GroupId | "other")[] = [...GROUPS.map((g) => g.id), ...(others.length ? ["other" as const] : [])];

  return (
    <section className="card card-pad settings-card">
      <CardTitle
        icon={<FileCog size={17} />}
        title={t.properties.title}
        hint={view.exists ? t.properties.hint : t.properties.notCreated}
      />
      <div className="segmented" role="tablist" aria-label={t.properties.title}>
        {groupIds.map((g) => (
          <button
            key={g}
            type="button"
            role="tab"
            aria-selected={group === g}
            data-selected={group === g || undefined}
            onClick={() => setGroup(g)}
          >
            {t.properties.groups[g]}
          </button>
        ))}
      </div>
      <div className="props">
        {group === "other"
          ? others.map((key) => (
              <div key={key} className="prop">
                <label className="prop-label" htmlFor={`prop-${key}`}>
                  <span className="mono">{key}</span>
                  {READ_ONLY.has(key) && <span className="prop-key">{t.properties.portElsewhere}</span>}
                </label>
                <div className="prop-control">
                  <input
                    id={`prop-${key}`}
                    className="input mono"
                    value={draft[key] ?? ""}
                    disabled={READ_ONLY.has(key)}
                    onChange={(e) => set(key, e.target.value)}
                  />
                </div>
              </div>
            ))
          : GROUPS.find((g) => g.id === group)!.fields.map(renderField)}
      </div>
      {dirty && (
        <div className="button-row">
          <button className="btn btn-primary btn-sm" onClick={save} disabled={saving || errors.length > 0}>
            {saving ? t.common.saving : t.properties.save}
          </button>
          <button className="btn btn-ghost btn-sm" onClick={() => setDraft(base)} disabled={saving}>
            <Undo2 size={14} />
            {t.common.discard}
          </button>
          {errors.length > 0 && (
            <span className="error-text">
              {t.properties.invalid(
                errors.map((f) => (t.properties.labels as Record<string, string>)[f.key] ?? f.key).join(", "),
              )}
            </span>
          )}
        </div>
      )}
    </section>
  );
}
