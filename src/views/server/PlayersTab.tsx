import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";
import {
  Ban,
  Crown,
  Gavel,
  ListChecks,
  LogOut,
  ShieldOff,
  UserMinus,
  UserPlus,
  UserRoundCheck,
  Users,
} from "lucide-react";
import { Avatar } from "../../components/Avatar";
import { useConfirm } from "../../components/Dialog";
import { useToast } from "../../components/Toasts";
import { Banner } from "../../components/ui";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import { formatDateTime } from "../../lib/format";
import { sleep, validPlayerName } from "../../lib/serverSettings";
import type { PlayerAction, PlayerLists, ServerSummary } from "../../lib/types";

/** Minecraft guarda las fechas como "2026-10-22 15:00:00 +0200". */
function parseMcDate(text: string): Date | null {
  const m = /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}) ([+-]\d{2})(\d{2})$/.exec(text.trim());
  if (!m) return null;
  const date = new Date(`${m[1]}T${m[2]}${m[3]}:${m[4]}`);
  return Number.isNaN(date.getTime()) ? null : date;
}

function ListCard(props: { icon: ReactNode; title: string; count: number; empty: string; children: ReactNode }) {
  return (
    <section className="card list-card">
      <header className="card-header">
        <span className="card-title">
          {props.icon}
          {props.title}
        </span>
        <span className="count">{props.count}</span>
      </header>
      {props.count === 0 ? <p className="list-empty">{props.empty}</p> : <ul className="rows">{props.children}</ul>}
    </section>
  );
}

function IconButton(props: { label: string; onClick: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <button
      className="btn btn-ghost btn-icon btn-sm"
      title={props.label}
      aria-label={props.label}
      onClick={props.onClick}
      disabled={props.disabled}
    >
      {props.children}
    </button>
  );
}

interface Props {
  server: ServerSummary;
  onError: (e: unknown) => void;
}

/** Pestaña Jugadores: lista blanca, operadores, baneos y expulsiones. */
export function PlayersTab({ server, onError }: Props) {
  const t = useT();
  const toast = useToast();
  const id = server.config.id;
  const running = server.status === "running";
  const [lists, setLists] = useState<PlayerLists | null>(null);
  const [properties, setProperties] = useState<Record<string, string> | null>(null);
  const [name, setName] = useState("");
  const [working, setWorking] = useState(false);
  const [dialog, confirm] = useConfirm();

  const load = useCallback(() => {
    api.playerLists(id).then(setLists).catch(onError);
    api
      .getProperties(id)
      .then((v) => setProperties(v.values))
      .catch(() => setProperties(null));
  }, [id, onError]);

  // Al entrar o salir alguien cambia la lista de conocidos.
  useEffect(() => {
    load();
  }, [load, server.players.length]);

  const act = async (action: PlayerAction, target: string, done?: string) => {
    setWorking(true);
    try {
      await api.playerAction(id, action, target);
      if (done) toast({ tone: "success", title: done });
      // Encendido, el servidor guarda sus listas un instante después del comando.
      if (running) await sleep(700);
      load();
    } catch (e) {
      onError(e);
    } finally {
      setWorking(false);
    }
  };

  const ban = async (target: string) => {
    const reason = await confirm({
      title: t.players.banTitle(target),
      message: t.players.banMessage,
      confirmLabel: t.players.ban,
      danger: true,
      input: { label: t.players.reason, placeholder: t.players.reasonPlaceholder },
    });
    if (reason !== null) act({ kind: "ban", reason }, target, t.players.banned(target));
  };

  const trimmed = name.trim();
  const nameOk = validPlayerName(trimmed);
  const add = (e: FormEvent) => {
    e.preventDefault();
    if (!nameOk) return;
    act({ kind: "whitelistAdd" }, trimmed, t.players.added(trimmed)).then(() => setName(""));
  };

  const whitelistOff = properties?.["white-list"] === "false";
  const enableWhitelist = async () => {
    try {
      await api.saveProperties(id, { "white-list": "true", "enforce-whitelist": "true" });
      if (running) await api.sendCommand(id, "whitelist on");
      toast({ tone: "success", title: t.overview.secured });
      load();
    } catch (e) {
      onError(e);
    }
  };

  const isOp = (n: string) => lists?.ops.some((o) => o.name.toLowerCase() === n.toLowerCase()) ?? false;
  const isWhitelisted = (n: string) => lists?.whitelist.some((w) => w.name.toLowerCase() === n.toLowerCase()) ?? false;
  const online = new Set(server.players.map((p) => p.toLowerCase()));
  const expires = (text: string) => {
    if (text === "forever") return t.players.forever;
    const date = parseMcDate(text);
    return t.players.until(date ? formatDateTime(date) : text);
  };
  const known = lists?.known.filter((k) => !isWhitelisted(k.name)) ?? [];

  return (
    <div className="tab-body">
      {dialog}
      {whitelistOff && (
        <Banner
          tone="warning"
          action={
            <button className="btn btn-secondary btn-sm" onClick={enableWhitelist}>
              {t.common.enable}
            </button>
          }
        >
          {t.overview.whitelistOff}
        </Banner>
      )}

      <section className="card card-pad add-player">
        <form className="add-player-form" onSubmit={add}>
          <label className="field">
            <span className="field-label">
              <UserPlus size={14} />
              {t.players.addTitle}
            </span>
            <input
              className="input"
              value={name}
              maxLength={16}
              placeholder={t.players.namePlaceholder}
              onChange={(e) => setName(e.target.value)}
              aria-invalid={trimmed.length > 0 && !nameOk}
              spellCheck={false}
              autoComplete="off"
            />
          </label>
          <div className="add-player-actions">
            <button className="btn btn-primary" type="submit" disabled={!nameOk || working}>
              <ListChecks size={15} />
              {t.players.addWhitelist}
            </button>
            <button
              className="btn btn-secondary"
              type="button"
              disabled={!nameOk || working}
              onClick={() => act({ kind: "op" }, trimmed, t.players.madeOp(trimmed)).then(() => setName(""))}
            >
              <Crown size={15} />
              {t.players.makeOp}
            </button>
            <button
              className="btn btn-ghost"
              type="button"
              disabled={!nameOk || working}
              onClick={() => ban(trimmed).then(() => setName(""))}
            >
              <Gavel size={15} />
              {t.players.ban}
            </button>
          </div>
        </form>
        <p className="field-hint">
          {trimmed.length > 0 && !nameOk ? t.players.invalidName : running ? t.players.hintRunning : t.players.hint}
        </p>
      </section>

      <div className="list-grid">
        <ListCard
          icon={<ListChecks size={16} />}
          title={t.players.whitelist}
          count={lists?.whitelist.length ?? 0}
          empty={t.players.whitelistEmpty}
        >
          {lists?.whitelist.map((w) => (
            <li key={w.uuid || w.name} className="row">
              <Avatar name={w.name} size={28} />
              <span className="row-main">
                <span className="row-title">{w.name}</span>
                <span className="row-tags">
                  {online.has(w.name.toLowerCase()) && (
                    <span className="tag" data-tone="good">
                      {t.status.running}
                    </span>
                  )}
                  {isOp(w.name) && <span className="tag">{t.players.opTag}</span>}
                </span>
              </span>
              <span className="row-actions">
                {running && online.has(w.name.toLowerCase()) && (
                  <IconButton
                    label={t.playersPanel.kick}
                    onClick={() => act({ kind: "kick", reason: "" }, w.name)}
                    disabled={working}
                  >
                    <LogOut size={14} />
                  </IconButton>
                )}
                {isOp(w.name) ? (
                  <IconButton
                    label={t.players.removeOp}
                    onClick={() => act({ kind: "deop" }, w.name)}
                    disabled={working}
                  >
                    <ShieldOff size={14} />
                  </IconButton>
                ) : (
                  <IconButton
                    label={t.players.makeOp}
                    onClick={() => act({ kind: "op" }, w.name, t.players.madeOp(w.name))}
                    disabled={working}
                  >
                    <Crown size={14} />
                  </IconButton>
                )}
                <IconButton
                  label={t.players.removeWhitelist}
                  onClick={() => act({ kind: "whitelistRemove" }, w.name)}
                  disabled={working}
                >
                  <UserMinus size={14} />
                </IconButton>
              </span>
            </li>
          ))}
        </ListCard>

        <ListCard
          icon={<Crown size={16} />}
          title={t.players.ops}
          count={lists?.ops.length ?? 0}
          empty={t.players.opsEmpty}
        >
          {lists?.ops.map((o) => (
            <li key={o.uuid || o.name} className="row">
              <Avatar name={o.name} size={28} />
              <span className="row-main">
                <span className="row-title">{o.name}</span>
                <span className="subtle">{t.players.level(o.level)}</span>
              </span>
              <span className="row-actions">
                <IconButton label={t.players.removeOp} onClick={() => act({ kind: "deop" }, o.name)} disabled={working}>
                  <ShieldOff size={14} />
                </IconButton>
              </span>
            </li>
          ))}
        </ListCard>

        <ListCard
          icon={<Gavel size={16} />}
          title={t.players.bannedPlayers}
          count={lists?.bannedPlayers.length ?? 0}
          empty={t.players.bannedEmpty}
        >
          {lists?.bannedPlayers.map((b) => (
            <li key={b.uuid || b.name} className="row">
              <Avatar name={b.name} size={28} />
              <span className="row-main">
                <span className="row-title">{b.name}</span>
                <span className="subtle">{[b.reason, expires(b.expires)].filter(Boolean).join(" · ")}</span>
              </span>
              <span className="row-actions">
                <button
                  className="btn btn-secondary btn-sm"
                  onClick={() => act({ kind: "pardon" }, b.name, t.players.pardoned(b.name))}
                  disabled={working}
                >
                  {t.players.pardon}
                </button>
              </span>
            </li>
          ))}
        </ListCard>

        <ListCard
          icon={<Ban size={16} />}
          title={t.players.bannedIps}
          count={lists?.bannedIps.length ?? 0}
          empty={t.players.bannedIpsEmpty}
        >
          {lists?.bannedIps.map((b) => (
            <li key={b.ip} className="row">
              <span className="row-icon">
                <Ban size={15} />
              </span>
              <span className="row-main">
                <span className="row-title mono">{b.ip}</span>
                <span className="subtle">{[b.reason, expires(b.expires)].filter(Boolean).join(" · ")}</span>
              </span>
              <span className="row-actions">
                <button
                  className="btn btn-secondary btn-sm"
                  onClick={() => act({ kind: "pardonIp" }, b.ip, t.players.unblocked(b.ip))}
                  disabled={working}
                >
                  {t.players.unblock}
                </button>
              </span>
            </li>
          ))}
        </ListCard>

        {known.length > 0 && (
          <ListCard icon={<Users size={16} />} title={t.players.known} count={known.length} empty="">
            {known.map((k) => (
              <li key={k.uuid || k.name} className="row">
                <Avatar name={k.name} size={28} />
                <span className="row-main">
                  <span className="row-title">{k.name}</span>
                  {online.has(k.name.toLowerCase()) && (
                    <span className="row-tags">
                      <span className="tag" data-tone="good">
                        {t.status.running}
                      </span>
                    </span>
                  )}
                </span>
                <span className="row-actions">
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() => act({ kind: "whitelistAdd" }, k.name, t.players.added(k.name))}
                    disabled={working}
                  >
                    <UserRoundCheck size={14} />
                    {t.players.addWhitelistShort}
                  </button>
                  <IconButton label={t.players.ban} onClick={() => ban(k.name)} disabled={working}>
                    <Gavel size={14} />
                  </IconButton>
                </span>
              </li>
            ))}
          </ListCard>
        )}
      </div>
    </div>
  );
}
