import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowUpCircle,
  Bell,
  CheckCircle2,
  CircleSlash,
  Coffee,
  ExternalLink,
  FolderOpen,
  Globe2,
  HardDrive,
  KeyRound,
  Languages,
  Plug,
  RefreshCw,
  Send,
} from "lucide-react";
import { useConfirm } from "../components/Dialog";
import { Logo } from "../components/Logo";
import { useToast } from "../components/Toasts";
import { Banner, CardTitle, SwitchRow } from "../components/ui";
import { LANG_NAMES, LANGS, useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatGb } from "../lib/format";
import type {
  AppInfo,
  AppSettings,
  AppUpdate,
  Claim,
  HardwareInfo,
  JavaRuntime,
  Lang,
  NotificationSettings,
  SecretKey,
  SettingsView,
} from "../lib/types";

interface Props {
  hardware: HardwareInfo | null;
  appInfo: AppInfo | null;
  update: AppUpdate | null;
  onUpdateFound: (update: AppUpdate | null) => void;
  onAppInfoChanged: () => void;
  onError: (e: unknown) => void;
}

const EVENTS: Exclude<keyof NotificationSettings, "desktop" | "discord">[] = [
  "playerJoin",
  "playerLeave",
  "serverReady",
  "serverCrash",
  "security",
  "backupFailed",
];

/** Lo que tarda como mucho la vinculación con playit.gg antes de rendirse. */
const CLAIM_TIMEOUT_MS = 10 * 60_000;

/** Campo para guardar un secreto en el llavero sin volver a mostrarlo. */
function SecretField(props: {
  saved: boolean;
  label: string;
  placeholder: string;
  savedText: string;
  onSave: (value: string) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const { t } = useI18n();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
      setValue("");
    } finally {
      setBusy(false);
    }
  };
  if (props.saved) {
    return (
      <div className="secret-saved">
        <span className="integration-state" data-on>
          <KeyRound size={15} />
          {props.savedText}
        </span>
        <button className="btn btn-ghost btn-sm" onClick={() => run(props.onDelete)} disabled={busy}>
          {t.common.remove}
        </button>
      </div>
    );
  }
  return (
    <form
      className="secret-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim()) run(() => props.onSave(value.trim()));
      }}
    >
      <input
        className="input mono"
        type="password"
        value={value}
        placeholder={props.placeholder}
        onChange={(e) => setValue(e.target.value)}
        autoComplete="off"
        aria-label={props.label}
      />
      <button className="btn btn-secondary" type="submit" disabled={busy || !value.trim()}>
        {t.common.save}
      </button>
    </form>
  );
}

export function Settings({ hardware, appInfo, update, onUpdateFound, onAppInfoChanged, onError }: Props) {
  const { t, setLang } = useI18n();
  const toast = useToast();
  const [view, setView] = useState<SettingsView | null>(null);
  const [runtimes, setRuntimes] = useState<JavaRuntime[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [claim, setClaim] = useState<Claim | null>(null);
  const [claimWaiting, setClaimWaiting] = useState(false);
  const [dialog, confirm] = useConfirm();
  const claimStarted = useRef(0);

  useEffect(() => {
    api.getSettings().then(setView).catch(onError);
    api.javaRuntimes().then(setRuntimes).catch(onError);
  }, [onError]);

  const save = useCallback(
    async (settings: AppSettings) => {
      try {
        const next = await api.saveSettings(settings);
        setView(next);
        setLang(next.language);
        onAppInfoChanged();
      } catch (e) {
        onError(e);
      }
    },
    [onError, onAppInfoChanged, setLang],
  );

  const setSecret = async (key: SecretKey, value: string) => {
    try {
      setView(await api.setSecret(key, value));
      toast({ tone: "success", title: t.settings.secretSaved });
      onAppInfoChanged();
    } catch (e) {
      onError(e);
    }
  };

  const deleteSecret = async (key: SecretKey) => {
    try {
      setView(await api.deleteSecret(key));
      onAppInfoChanged();
    } catch (e) {
      onError(e);
    }
  };

  // Vinculación con playit.gg: se abre su web y se pregunta cada 2 s si ya aceptó.
  useEffect(() => {
    if (!claim) return;
    let stopped = false;
    const poll = async () => {
      if (stopped) return;
      if (Date.now() - claimStarted.current > CLAIM_TIMEOUT_MS) {
        setClaim(null);
        onError(t.settings.playitTimeout);
        return;
      }
      try {
        const progress = await api.playitClaimPoll(claim.code);
        if (stopped) return;
        setClaimWaiting(progress.status === "waitingForUser");
        if (progress.linked) {
          setClaim(null);
          toast({ tone: "success", title: t.settings.playitLinked });
          setView(await api.getSettings());
          return;
        }
      } catch (e) {
        if (stopped) return;
        setClaim(null);
        onError(e);
        return;
      }
      window.setTimeout(poll, 2000);
    };
    const timer = window.setTimeout(poll, 2000);
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [claim, onError, toast, t]);

  const startClaim = async () => {
    try {
      claimStarted.current = Date.now();
      setClaimWaiting(false);
      setClaim(await api.playitClaimStart());
    } catch (e) {
      onError(e);
    }
  };

  const checkUpdate = async () => {
    setChecking(true);
    try {
      onUpdateFound(await api.checkAppUpdate());
      setChecked(true);
    } catch (e) {
      onError(e);
    } finally {
      setChecking(false);
    }
  };

  const install = async () => {
    if (!update) return;
    const ok = await confirm({
      title: t.settings.installTitle(update.version),
      message: t.settings.installMessage,
      confirmLabel: t.settings.install,
    });
    if (ok === null) return;
    setInstalling(true);
    try {
      await api.installAppUpdate();
    } catch (e) {
      onError(e);
      setInstalling(false);
    }
  };

  const settings = view?.settings;
  const notifications = settings?.notifications;
  const setNotifications = (patch: Partial<NotificationSettings>) =>
    settings && notifications && save({ ...settings, notifications: { ...notifications, ...patch } });

  return (
    <div className="page settings">
      {dialog}
      <header className="page-header">
        <div className="page-heading">
          <h1>{t.settings.title}</h1>
          <p className="muted">{t.settings.subtitle}</p>
        </div>
      </header>

      <div className="settings-grid">
        <div className="settings-col">
          <section className="card card-pad settings-card">
            <CardTitle icon={<Languages size={17} />} title={t.settings.language} />
            <select
              className="select"
              value={settings?.language ?? "auto"}
              disabled={!settings}
              onChange={(e) =>
                settings && save({ ...settings, language: e.target.value === "auto" ? null : (e.target.value as Lang) })
              }
              aria-label={t.settings.language}
            >
              <option value="auto">{t.settings.languageAuto}</option>
              {LANGS.map((l) => (
                <option key={l} value={l}>
                  {LANG_NAMES[l]}
                </option>
              ))}
            </select>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<Bell size={17} />} title={t.settings.notifications} hint={t.settings.notificationsHint} />
            {notifications && view && (
              <>
                <SwitchRow
                  label={t.settings.desktop}
                  checked={notifications.desktop}
                  onChange={(desktop) => setNotifications({ desktop })}
                />
                <SwitchRow
                  label={t.settings.discord}
                  hint={t.settings.discordHint}
                  checked={notifications.discord}
                  onChange={(discord) => setNotifications({ discord })}
                />
                {notifications.discord && (
                  <SecretField
                    saved={view.secrets.discordWebhook}
                    label={t.settings.webhook}
                    placeholder="https://discord.com/api/webhooks/…"
                    savedText={t.settings.webhookSaved}
                    onSave={(v) => setSecret("discordWebhook", v)}
                    onDelete={() => deleteSecret("discordWebhook")}
                  />
                )}
                <div className="checks">
                  <span className="field-label">{t.settings.events}</span>
                  {EVENTS.map((key) => (
                    <label key={key} className="check check-sm">
                      <input
                        type="checkbox"
                        checked={notifications[key]}
                        onChange={(e) => setNotifications({ [key]: e.target.checked })}
                      />
                      <span>{t.settings.eventNames[key]}</span>
                    </label>
                  ))}
                </div>
                <div>
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() =>
                      api
                        .testNotification()
                        .then(() => toast({ tone: "success", title: t.settings.testSent }))
                        .catch(onError)
                    }
                  >
                    <Send size={14} />
                    {t.settings.test}
                  </button>
                </div>
              </>
            )}
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<Globe2 size={17} />} title={t.settings.playit} hint={t.settings.playitHint} />
            {view?.secrets.playitAgent ? (
              <div className="secret-saved">
                <span className="integration-state" data-on>
                  <CheckCircle2 size={15} />
                  {t.settings.playitLinked}
                </span>
                <button className="btn btn-ghost btn-sm" onClick={() => deleteSecret("playitAgent")}>
                  {t.settings.unlink}
                </button>
              </div>
            ) : claim ? (
              <div className="claim">
                <span className="claim-status">
                  <span className="spinner" />
                  {claimWaiting ? t.settings.playitAccept : t.settings.playitOpen}
                </span>
                <span className="field-hint">{t.settings.playitCode(claim.code)}</span>
                <div className="button-row">
                  <button className="btn btn-secondary btn-sm" onClick={() => api.openUrl(claim.url).catch(onError)}>
                    <ExternalLink size={14} />
                    {t.settings.openAgain}
                  </button>
                  <button className="btn btn-ghost btn-sm" onClick={() => setClaim(null)}>
                    {t.common.cancel}
                  </button>
                </div>
              </div>
            ) : (
              <div>
                <button className="btn btn-primary btn-sm" onClick={startClaim} disabled={!view}>
                  <Globe2 size={14} />
                  {t.settings.link}
                </button>
              </div>
            )}
          </section>
        </div>

        <div className="settings-col">
          <section className="card card-pad settings-card">
            <CardTitle icon={<RefreshCw size={17} />} title={t.settings.updates} />
            {settings && (
              <SwitchRow
                label={t.settings.autoCheck}
                checked={settings.updates.autoCheck}
                onChange={(autoCheck) => save({ ...settings, updates: { autoCheck } })}
              />
            )}
            {update ? (
              <div className="update-box">
                <strong>
                  <ArrowUpCircle size={16} />
                  {t.settings.available(update.version)}
                </strong>
                {update.notes && <p className="update-notes selectable">{update.notes}</p>}
                {update.canInstall ? (
                  <div>
                    <button className="btn btn-primary btn-sm" onClick={install} disabled={installing}>
                      {installing ? t.settings.installing : t.settings.install}
                    </button>
                  </div>
                ) : (
                  <Banner>{t.settings.manualUpdate}</Banner>
                )}
              </div>
            ) : (
              <div className="button-row">
                <button className="btn btn-secondary btn-sm" onClick={checkUpdate} disabled={checking}>
                  <RefreshCw size={14} />
                  {checking ? t.settings.checking : t.settings.checkNow}
                </button>
                {checked && (
                  <span className="inline-ok">
                    <CheckCircle2 size={14} />
                    {t.settings.upToDate}
                  </span>
                )}
              </div>
            )}
            <span className="field-hint">{t.settings.version(appInfo?.version ?? "…")}</span>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<Plug size={17} />} title={t.settings.sources} />
            <ul className="integrations">
              <li>
                <span>Modrinth</span>
                <span className="integration-state" data-on>
                  <CheckCircle2 size={15} /> {t.settings.active}
                </span>
              </li>
              <li>
                <span>CurseForge</span>
                {appInfo?.curseforgeEnabled ? (
                  <span className="integration-state" data-on>
                    <CheckCircle2 size={15} /> {t.settings.active}
                  </span>
                ) : (
                  <span className="integration-state">
                    <CircleSlash size={15} /> {t.settings.needsKey}
                  </span>
                )}
              </li>
            </ul>
            {view && (
              <SecretField
                saved={view.secrets.curseforgeApiKey}
                label={t.settings.curseforgeKey}
                placeholder={t.settings.curseforgeKey}
                savedText={t.settings.keySaved}
                onSave={(v) => setSecret("curseforgeApiKey", v)}
                onDelete={() => deleteSecret("curseforgeApiKey")}
              />
            )}
            <p className="field-hint">
              {view && !view.secrets.curseforgeApiKey && appInfo?.curseforgeEnabled
                ? t.settings.builtInKey
                : t.settings.curseforgeHint}{" "}
              <button
                className="link-button"
                onClick={() => api.openUrl("https://console.curseforge.com/").catch(onError)}
              >
                console.curseforge.com
              </button>
            </p>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<HardDrive size={17} />} title={t.settings.computer} />
            <dl className="facts">
              <div>
                <dt>{t.settings.cpu}</dt>
                <dd>{hardware ? t.common.threads(hardware.logicalCpus) : "…"}</dd>
              </div>
              <div>
                <dt>{t.settings.ram}</dt>
                <dd>{hardware ? formatGb(hardware.totalMemoryMb) : "…"}</dd>
              </div>
              <div>
                <dt>{t.settings.ramMax}</dt>
                <dd>{hardware ? formatGb(hardware.recommendedMaxMb) : "…"}</dd>
              </div>
            </dl>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<Coffee size={17} />} title={t.settings.java} />
            {runtimes === null ? (
              <p className="muted">{t.common.loading}</p>
            ) : runtimes.length === 0 ? (
              <p className="muted">{t.settings.javaNone}</p>
            ) : (
              <ul className="runtime-list">
                {runtimes.map((rt) => (
                  <li key={rt.major}>
                    <span className="runtime-badge">{t.common.java(rt.major)}</span>
                    <span className="muted mono">{rt.releaseName}</span>
                  </li>
                ))}
              </ul>
            )}
            <p className="field-hint">{t.settings.javaHint}</p>
          </section>

          <section className="card card-pad settings-card">
            <CardTitle icon={<FolderOpen size={17} />} title={t.settings.data} />
            <p className="muted">{t.settings.dataHint}</p>
            <code className="path selectable">{appInfo?.dataDir ?? "…"}</code>
            {view && (
              <p className="field-hint">
                <KeyRound size={13} className="inline-icon" />{" "}
                {view.secretBackend === "keyring" ? t.settings.keyring : t.settings.keyFile}
              </p>
            )}
            <div>
              <button className="btn btn-secondary btn-sm" onClick={() => api.openDataFolder().catch(onError)}>
                <FolderOpen size={14} />
                {t.common.openFolder}
              </button>
            </div>
          </section>
        </div>

        <section className="card card-pad about">
          <Logo size={34} />
          <div className="about-text">
            <strong>Micra {appInfo?.version ?? ""}</strong>
            <span className="muted">{t.settings.about}</span>
          </div>
        </section>
      </div>
    </div>
  );
}
