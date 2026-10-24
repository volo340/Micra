import { useCallback, useEffect, useState } from "react";
import {
  CalendarClock,
  ExternalLink,
  Globe2,
  Map as MapIcon,
  MemoryStick,
  ShieldCheck,
  Trash2,
  Undo2,
} from "lucide-react";
import { useConfirm } from "../../components/Dialog";
import { CoresField, MemoryField, PortField, validPort } from "../../components/ResourceFields";
import { useToast } from "../../components/Toasts";
import { Banner, CardTitle, SwitchRow } from "../../components/ui";
import { useT } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { sameValue, settingsOf } from "../../lib/serverSettings";
import type { HardwareInfo, NetworkInfo, PropertiesView, ServerSettings, ServerSummary } from "../../lib/types";
import { PropertiesEditor } from "./PropertiesEditor";

const RESTART_EVERY = [4, 6, 8, 12, 24, 48];
const IDLE_MINUTES = [5, 10, 15, 30, 60, 120];
const WINDOWS = [30, 60, 120, 300, 600];
const BANS = [15, 60, 360, 1440, 0];

const validTime = (s: string) => /^([01]\d|2[0-3]):[0-5]\d$/.test(s);
const withCurrent = (list: number[], value: number) => [...new Set([...list, value])].sort((a, b) => a - b);

interface Props {
  server: ServerSummary;
  hardware: HardwareInfo | null;
  network: NetworkInfo | null;
  platform: "windows" | "linux";
  onError: (e: unknown) => void;
  onSaved: () => void;
  onOpenAppSettings: () => void;
  onDeleted: () => void;
}

/** Pestaña Ajustes del servidor: recursos, red, reinicios, seguridad, mapa, server.properties y borrarlo. */
export function ServerSettingsTab(props: Props) {
  const { server, hardware, network, onError, onSaved, onOpenAppSettings, onDeleted } = props;
  const t = useT();
  const toast = useToast();
  const { config, status, live } = server;
  const id = config.id;
  const stopped = status === "stopped" || status === "crashed";
  const running = status === "running";
  const saved = settingsOf(config);
  const savedKey = JSON.stringify(saved);
  const [draft, setDraft] = useState<ServerSettings>(saved);
  const [saving, setSaving] = useState(false);
  const [properties, setProperties] = useState<PropertiesView | null>(null);
  const [tunnelLinked, setTunnelLinked] = useState<boolean | null>(null);
  const [publicIp, setPublicIp] = useState<string | null>(null);
  const [mapAccept, setMapAccept] = useState(false);
  const [mapWorking, setMapWorking] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [dialog, confirm] = useConfirm();

  useEffect(() => setDraft(JSON.parse(savedKey)), [savedKey]);

  const loadProperties = useCallback(
    () =>
      api
        .getProperties(id)
        .then(setProperties)
        .catch((e) => {
          setProperties({ values: {}, exists: false, rules: {}, removed: [], pending: [] });
          onError(e);
        }),
    [id, onError],
  );

  useEffect(() => {
    loadProperties();
    api
      .getSettings()
      .then((v) => setTunnelLinked(v.secrets.playitAgent))
      .catch(() => setTunnelLinked(false));
  }, [loadProperties]);

  const patch = (p: Partial<ServerSettings>) => setDraft((d) => ({ ...d, ...p }));
  const schedule = draft.schedule;
  const guard = draft.security.guard;
  const setGuard = (g: Partial<typeof guard>) => patch({ security: { ...draft.security, guard: { ...guard, ...g } } });
  const cpus = hardware?.logicalCpus ?? 4;
  const cores = draft.cpu.cores ?? [];

  const problems = [
    !validPort(draft.port) && t.resources.portInvalid,
    draft.cpu.cores !== null && draft.cpu.cores.length === 0 && t.resources.pickOne,
    schedule.dailyRestart !== null && !validTime(schedule.dailyRestart) && t.serverSettings.timeInvalid,
    guard.enabled && (!(guard.maxAttempts >= 2) || guard.maxAttempts > 1000) && t.serverSettings.attemptsInvalid,
    guard.enabled && !(guard.maxPerIp >= 0 && guard.maxPerIp <= 100) && t.serverSettings.perIpInvalid,
    config.map.enabled &&
      (!validPort(draft.mapPort) || draft.mapPort === draft.port) &&
      t.serverSettings.mapPortInvalid,
  ].filter((p): p is string => Boolean(p));
  const dirty = !sameValue(draft, saved);

  const save = async () => {
    setSaving(true);
    try {
      const result = await api.updateServerSettings(id, draft);
      toast(
        result.restartNeeded
          ? { tone: "info", title: t.common.saved, message: t.serverSettings.restartNeeded }
          : { tone: "success", title: t.common.saved },
      );
      onSaved();
    } catch (e) {
      onError(e);
    } finally {
      setSaving(false);
    }
  };

  const mapAction = async (action: () => Promise<unknown>, done: string) => {
    setMapWorking(true);
    try {
      await action();
      toast({ tone: "success", title: done });
      onSaved();
    } catch (e) {
      onError(e);
    } finally {
      setMapWorking(false);
    }
  };

  const showPublicIp = () =>
    api
      .publicIp()
      .then(setPublicIp)
      .catch((e) => setPublicIp(errorMessage(e)));

  const sandbox = live.sandbox;
  const sandboxState =
    !running || !sandbox
      ? null
      : sandbox.kind === "landlock"
        ? t.serverSettings.sandboxLandlock(sandbox.abi)
        : sandbox.kind === "jobObject"
          ? sandbox.lowIntegrity
            ? t.serverSettings.sandboxJobLow
            : t.serverSettings.sandboxJob
          : sandbox.kind === "unavailable"
            ? t.serverSettings.sandboxUnavailable(sandbox.reason)
            : t.serverSettings.sandboxOff;
  const windows = props.platform === "windows";
  const rconOn = properties?.values["enable-rcon"] === "true";
  const lan = network?.localIp;

  const deleteServer = async () => {
    const backups = await api
      .listBackups(id)
      .then((list) => list.length)
      .catch(() => 0);
    let withBackups = true;
    const ok = await confirm({
      title: t.serverSettings.deleteTitle(config.name),
      message: t.serverSettings.deleteMessage,
      confirmLabel: t.serverSettings.delete,
      danger: true,
      check:
        backups > 0
          ? {
              label: t.serverSettings.deleteBackups(backups),
              hint: t.serverSettings.keepBackupsHint,
              defaultChecked: true,
              onChange: (checked) => (withBackups = checked),
            }
          : undefined,
    });
    if (ok === null) return;
    setDeleting(true);
    try {
      await api.deleteServer(id, withBackups);
      onDeleted();
    } catch (e) {
      onError(e);
      setDeleting(false);
    }
  };

  return (
    <div className="tab-body settings-tab">
      {dialog}
      <div className="settings-grid">
        <div className="settings-col">
          <section className="card card-pad settings-card">
            <CardTitle
              icon={<MemoryStick size={17} />}
              title={t.serverSettings.resources}
              hint={t.common.appliesOnRestart}
            />
            <MemoryField
              value={draft.memory.maxMb}
              hardware={hardware}
              onChange={(maxMb) => patch({ memory: { minMb: Math.min(draft.memory.minMb, maxMb), maxMb } })}
            />
            <CoresField
              allCores={draft.cpu.cores === null}
              cores={cores}
              cpus={cpus}
              onChange={(all, list) => patch({ cpu: { cores: all ? null : list } })}
            />
            <SwitchRow
              label={t.serverSettings.aikar}
              hint={t.serverSettings.aikarHint}
              checked={draft.useAikarFlags}
              onChange={(useAikarFlags) => patch({ useAikarFlags })}
            />
          </section>

          <section className="card card-pad settings-card">
            <CardTitle
              icon={<CalendarClock size={17} />}
              title={t.serverSettings.schedule}
              hint={t.serverSettings.scheduleHint}
            />
            <div className="switch-with-field">
              <SwitchRow
                label={t.serverSettings.daily}
                checked={schedule.dailyRestart !== null}
                onChange={(on) => patch({ schedule: { ...schedule, dailyRestart: on ? "05:00" : null } })}
              />
              {schedule.dailyRestart !== null && (
                <input
                  className="input input-time mono"
                  type="time"
                  value={schedule.dailyRestart}
                  onChange={(e) => patch({ schedule: { ...schedule, dailyRestart: e.target.value } })}
                  aria-label={t.serverSettings.dailyTime}
                  aria-invalid={!validTime(schedule.dailyRestart)}
                />
              )}
            </div>
            <div className="switch-with-field">
              <SwitchRow
                label={t.serverSettings.every}
                checked={schedule.restartEveryHours !== null}
                onChange={(on) => patch({ schedule: { ...schedule, restartEveryHours: on ? 12 : null } })}
              />
              {schedule.restartEveryHours !== null && (
                <select
                  className="select select-narrow"
                  value={schedule.restartEveryHours}
                  onChange={(e) => patch({ schedule: { ...schedule, restartEveryHours: Number(e.target.value) } })}
                  aria-label={t.serverSettings.every}
                >
                  {withCurrent(RESTART_EVERY, schedule.restartEveryHours).map((h) => (
                    <option key={h} value={h}>
                      {t.common.hours(h)}
                    </option>
                  ))}
                </select>
              )}
            </div>
            <div className="switch-with-field">
              <SwitchRow
                label={t.serverSettings.idle}
                hint={t.serverSettings.idleHint}
                checked={schedule.idleShutdownMinutes !== null}
                onChange={(on) => patch({ schedule: { ...schedule, idleShutdownMinutes: on ? 30 : null } })}
              />
              {schedule.idleShutdownMinutes !== null && (
                <select
                  className="select select-narrow"
                  value={schedule.idleShutdownMinutes}
                  onChange={(e) => patch({ schedule: { ...schedule, idleShutdownMinutes: Number(e.target.value) } })}
                  aria-label={t.serverSettings.idle}
                >
                  {withCurrent(IDLE_MINUTES, schedule.idleShutdownMinutes).map((m) => (
                    <option key={m} value={m}>
                      {t.common.minutes(m)}
                    </option>
                  ))}
                </select>
              )}
            </div>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<MapIcon size={17} />} title={t.serverSettings.map} hint={t.serverSettings.mapHint} />
            {config.loader.kind === "vanilla" ? (
              <p className="muted">{t.serverSettings.mapNeedsLoader}</p>
            ) : !config.map.enabled ? (
              <>
                <label className="check">
                  <input type="checkbox" checked={mapAccept} onChange={(e) => setMapAccept(e.target.checked)} />
                  <span className="muted">{t.serverSettings.mapAccept}</span>
                </label>
                <div className="button-row">
                  <button
                    className="btn btn-primary btn-sm"
                    disabled={!stopped || mapWorking}
                    title={stopped ? undefined : t.common.stopFirst}
                    onClick={() => mapAction(() => api.enableMap(id, mapAccept), t.serverSettings.mapEnabled)}
                  >
                    <MapIcon size={14} />
                    {mapWorking ? t.serverSettings.mapInstalling : t.serverSettings.mapEnable}
                  </button>
                  {!stopped && <span className="subtle">{t.common.stopFirst}</span>}
                </div>
              </>
            ) : (
              <>
                {!config.map.acceptedDownload && (
                  <Banner
                    tone="warning"
                    action={
                      <button
                        className="btn btn-secondary btn-sm"
                        onClick={() => mapAction(() => api.acceptMapDownload(id), t.serverSettings.mapAccepted)}
                      >
                        {t.overview.mapAccept}
                      </button>
                    }
                  >
                    {t.overview.mapNeedsAccept}
                  </Banner>
                )}
                <PortField
                  value={draft.mapPort}
                  onChange={(mapPort) => patch({ mapPort })}
                  hint={t.serverSettings.mapWhere(draft.mapPort, lan ?? null)}
                />
                <div className="button-row">
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() => api.openMap(id).catch(onError)}
                    disabled={!running}
                    title={running ? undefined : t.serverSettings.mapWhenRunning}
                  >
                    <ExternalLink size={14} />
                    {t.server.openMap}
                  </button>
                  <button
                    className="btn btn-ghost btn-sm"
                    disabled={!stopped || mapWorking}
                    title={stopped ? undefined : t.common.stopFirst}
                    onClick={() => mapAction(() => api.disableMap(id), t.serverSettings.mapDisabled)}
                  >
                    {t.serverSettings.mapDisable}
                  </button>
                </div>
              </>
            )}
          </section>
        </div>
        <div className="settings-col">
          <section className="card card-pad settings-card">
            <CardTitle
              icon={<Globe2 size={17} />}
              title={t.serverSettings.network}
              hint={t.serverSettings.networkHint}
            />
            <PortField value={draft.port} onChange={(port) => patch({ port })} hint={t.serverSettings.portHint} />
            <SwitchRow
              label={t.serverSettings.upnp}
              hint={t.serverSettings.upnpHint}
              checked={draft.network.upnp}
              onChange={(upnp) => patch({ network: { ...draft.network, upnp } })}
            />
            <SwitchRow
              label={t.serverSettings.tunnel}
              hint={
                tunnelLinked === false && !draft.network.tunnel
                  ? t.serverSettings.tunnelNeedsLink
                  : t.serverSettings.tunnelHint
              }
              checked={draft.network.tunnel}
              disabled={tunnelLinked === false && !draft.network.tunnel}
              onChange={(tunnel) => patch({ network: { ...draft.network, tunnel } })}
            />
            {tunnelLinked === false && (
              <div>
                <button className="btn btn-secondary btn-sm" onClick={onOpenAppSettings}>
                  {t.serverSettings.linkPlayit}
                </button>
              </div>
            )}
            {draft.network.tunnel && <Banner>{t.serverSettings.tunnelIpNote}</Banner>}
            <div className="button-row">
              <button className="btn btn-ghost btn-sm" onClick={showPublicIp}>
                {t.connect.showIp}
              </button>
              {publicIp && <span className="mono selectable">{publicIp}</span>}
            </div>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<ShieldCheck size={17} />} title={t.serverSettings.security} />
            <SwitchRow
              label={t.serverSettings.guard}
              hint={t.serverSettings.guardHint}
              checked={guard.enabled}
              onChange={(enabled) => setGuard({ enabled })}
            />
            {guard.enabled && (
              <div className="inline-fields">
                <label className="field">
                  <span className="field-label">{t.serverSettings.attempts}</span>
                  <input
                    className="input"
                    type="number"
                    min={2}
                    max={1000}
                    value={Number.isNaN(guard.maxAttempts) ? "" : guard.maxAttempts}
                    onChange={(e) => setGuard({ maxAttempts: e.target.valueAsNumber })}
                  />
                </label>
                <label className="field">
                  <span className="field-label">{t.serverSettings.window}</span>
                  <select
                    className="select"
                    value={guard.windowSecs}
                    onChange={(e) => setGuard({ windowSecs: Number(e.target.value) })}
                  >
                    {withCurrent(WINDOWS, guard.windowSecs).map((s) => (
                      <option key={s} value={s}>
                        {s % 60 === 0 ? t.common.minutes(s / 60) : t.common.seconds(s)}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span className="field-label">{t.serverSettings.banFor}</span>
                  <select
                    className="select"
                    value={guard.banMinutes}
                    onChange={(e) => setGuard({ banMinutes: Number(e.target.value) })}
                  >
                    {[...withCurrent(BANS.filter(Boolean), guard.banMinutes || 15), 0].map((m) => (
                      <option key={m} value={m}>
                        {m === 0 ? t.players.forever : m % 60 === 0 ? t.common.hours(m / 60) : t.common.minutes(m)}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span className="field-label">{t.serverSettings.perIp}</span>
                  <input
                    className="input"
                    type="number"
                    min={0}
                    max={100}
                    value={Number.isNaN(guard.maxPerIp) ? "" : guard.maxPerIp}
                    onChange={(e) => setGuard({ maxPerIp: e.target.valueAsNumber })}
                  />
                </label>
              </div>
            )}
            {guard.enabled && <span className="field-hint">{t.serverSettings.perIpHint}</span>}
            <SwitchRow
              label={t.serverSettings.sandbox}
              hint={windows ? t.serverSettings.sandboxHintWindows : t.serverSettings.sandboxHintLinux}
              checked={draft.security.sandbox}
              onChange={(on) => patch({ security: { ...draft.security, sandbox: on } })}
            />
            {windows && draft.security.sandbox && (
              <SwitchRow
                label={t.serverSettings.lowIntegrity}
                hint={t.serverSettings.lowIntegrityHint}
                checked={draft.security.lowIntegrity}
                onChange={(lowIntegrity) => patch({ security: { ...draft.security, lowIntegrity } })}
              />
            )}
            {sandboxState && <span className="field-hint">{sandboxState}</span>}
            <div className="fact-line">
              <span className="fact-label">RCON</span>
              <span className="muted">{rconOn ? t.serverSettings.rconOn : t.serverSettings.rconOff}</span>
            </div>
          </section>
        </div>
      </div>

      {properties && (
        <PropertiesEditor serverId={id} view={properties} running={running} onSaved={setProperties} onError={onError} />
      )}

      <section className="card card-pad settings-card danger-card">
        <CardTitle
          icon={<Trash2 size={17} />}
          title={t.serverSettings.danger}
          hint={t.serverSettings.dangerHint}
          aside={
            <button
              className="btn btn-danger btn-sm"
              onClick={deleteServer}
              disabled={!stopped || server.busy || deleting}
              title={stopped ? undefined : t.common.stopFirst}
            >
              <Trash2 size={14} />
              {deleting ? t.serverSettings.deleting : t.serverSettings.delete}
            </button>
          }
        />
        {!stopped && <span className="subtle">{t.common.stopFirst}</span>}
      </section>

      {(dirty || saving) && (
        <div className="save-bar" role="region" aria-label={t.common.unsaved}>
          <span className="save-bar-text">{problems[0] ?? t.common.unsaved}</span>
          <button className="btn btn-ghost btn-sm" onClick={() => setDraft(saved)} disabled={saving}>
            <Undo2 size={14} />
            {t.common.discard}
          </button>
          <button className="btn btn-primary btn-sm" onClick={save} disabled={saving || problems.length > 0}>
            {saving ? t.common.saving : t.common.save}
          </button>
        </div>
      )}
    </div>
  );
}
