import { useCallback, useEffect, useState } from "react";
import { Archive, DatabaseBackup, FolderOpen, History, RotateCcw, Save, Trash2 } from "lucide-react";
import { useConfirm } from "../../components/Dialog";
import { useToast } from "../../components/Toasts";
import { Banner, CardTitle, Progress, SwitchRow } from "../../components/ui";
import { useT } from "../../i18n";
import { api } from "../../lib/api";
import { formatBytes, formatDateTime, formatRelative } from "../../lib/format";
import { sameValue, saveServerSettings } from "../../lib/serverSettings";
import type { BackupInfo, BackupPolicy, BackupScope, ServerSummary } from "../../lib/types";
import { useTask } from "../../lib/useTask";

const INTERVALS = [1, 2, 3, 6, 12, 24];

interface Props {
  server: ServerSummary;
  onError: (e: unknown) => void;
  onSaved: () => void;
}

/** Pestaña Copias: copias manuales y automáticas, restaurar con un clic. */
export function BackupsTab({ server, onError, onSaved }: Props) {
  const t = useT();
  const toast = useToast();
  const { config, status, live } = server;
  const id = config.id;
  const stopped = status === "stopped" || status === "crashed";
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [policy, setPolicy] = useState<BackupPolicy>(config.backups);
  const [saving, setSaving] = useState(false);
  const { progress, run } = useTask(id);
  const [dialog, confirm] = useConfirm();
  const working = progress !== null || server.busy;

  const savedPolicy = JSON.stringify(config.backups);
  useEffect(() => setPolicy(JSON.parse(savedPolicy)), [savedPolicy]);

  const load = useCallback(
    () =>
      api
        .listBackups(id)
        .then(setBackups)
        .catch((e) => {
          setBackups([]);
          onError(e);
        }),
    [id, onError],
  );

  useEffect(() => {
    load();
  }, [load]);

  const create = async (scope: BackupScope) => {
    try {
      await run("backup", () => api.createBackup(id, scope));
      toast({ tone: "success", title: t.backups.created });
    } catch (e) {
      onError(e);
    }
    load();
  };

  const restore = async (b: BackupInfo) => {
    const ok = await confirm({
      title: t.backups.restoreTitle,
      message: t.backups.restoreMessage(formatDateTime(b.createdAt)),
      confirmLabel: t.backups.restore,
      danger: true,
    });
    if (ok === null) return;
    try {
      await run("restore", () => api.restoreBackup(id, b.fileName));
      toast({ tone: "success", title: t.backups.restored, message: t.backups.restoredHint });
    } catch (e) {
      onError(e);
    }
    load();
  };

  const remove = async (b: BackupInfo) => {
    const ok = await confirm({
      title: t.backups.deleteTitle,
      message: t.backups.deleteMessage(formatDateTime(b.createdAt)),
      confirmLabel: t.common.delete,
      danger: true,
    });
    if (ok === null) return;
    try {
      await api.deleteBackup(id, b.fileName);
    } catch (e) {
      onError(e);
    }
    load();
  };

  const savePolicy = async () => {
    setSaving(true);
    try {
      await saveServerSettings(config, { backups: policy });
      toast({ tone: "success", title: t.common.saved });
      onSaved();
    } catch (e) {
      onError(e);
    } finally {
      setSaving(false);
    }
  };

  const policyValid = !policy.enabled || (policy.intervalHours > 0 && policy.keep >= 1 && policy.keep <= 1000);
  const dirty = !sameValue(policy, config.backups);

  return (
    <div className="tab-body">
      {dialog}
      {live.lastBackupError && <Banner tone="critical">{t.overview.backupFailed(live.lastBackupError)}</Banner>}

      <div className="backups-top">
        <section className="card card-pad settings-card">
          <CardTitle icon={<DatabaseBackup size={17} />} title={t.backups.title} hint={t.backups.intro} />
          <div className="button-row">
            <button className="btn btn-primary" onClick={() => create("world")} disabled={working}>
              <Save size={15} />
              {t.backups.createWorld}
            </button>
            <button
              className="btn btn-secondary"
              onClick={() => create("full")}
              disabled={working || !stopped}
              title={stopped ? undefined : t.backups.fullNeedsStop}
            >
              <Archive size={15} />
              {t.backups.createFull}
            </button>
            <button className="btn btn-ghost" onClick={() => api.openBackupsFolder(id).catch(onError)}>
              <FolderOpen size={15} />
              {t.common.openFolder}
            </button>
          </div>
          {!stopped && <p className="field-hint">{t.backups.fullNeedsStop}</p>}
          {progress && (
            <Progress
              done={progress.done}
              total={progress.total}
              label={progress.task === "restore" ? t.backups.restoring : t.backups.copying}
            />
          )}
        </section>

        <section className="card card-pad settings-card">
          <CardTitle icon={<History size={17} />} title={t.backups.autoTitle} />
          <SwitchRow
            label={t.backups.autoLabel}
            hint={t.backups.autoHint}
            checked={policy.enabled}
            onChange={(enabled) => setPolicy({ ...policy, enabled })}
          />
          <div className="inline-fields">
            <label className="field">
              <span className="field-label">{t.backups.every}</span>
              <select
                className="select"
                value={policy.intervalHours}
                disabled={!policy.enabled}
                onChange={(e) => setPolicy({ ...policy, intervalHours: Number(e.target.value) })}
              >
                {[...new Set([...INTERVALS, policy.intervalHours])]
                  .sort((a, b) => a - b)
                  .map((h) => (
                    <option key={h} value={h}>
                      {t.common.hours(h)}
                    </option>
                  ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">{t.backups.keep}</span>
              <input
                className="input"
                type="number"
                min={1}
                max={1000}
                value={Number.isNaN(policy.keep) ? "" : policy.keep}
                disabled={!policy.enabled}
                onChange={(e) => setPolicy({ ...policy, keep: e.target.valueAsNumber })}
                aria-invalid={!policyValid}
              />
            </label>
          </div>
          {dirty && (
            <div className="button-row">
              <button className="btn btn-primary btn-sm" onClick={savePolicy} disabled={saving || !policyValid}>
                {saving ? t.common.saving : t.common.save}
              </button>
              <button className="btn btn-ghost btn-sm" onClick={() => setPolicy(config.backups)} disabled={saving}>
                {t.common.discard}
              </button>
            </div>
          )}
        </section>
      </div>

      <section className="card list-card">
        <header className="card-header">
          <span className="card-title">
            <Archive size={16} />
            {t.backups.list}
          </span>
          <span className="card-header-aside">
            {!stopped && !!backups?.length && <span className="field-hint">{t.backups.restoreNeedsStop}</span>}
            <span className="count">{backups?.length ?? 0}</span>
          </span>
        </header>
        {backups === null ? (
          <p className="list-empty">{t.common.loading}</p>
        ) : backups.length === 0 ? (
          <div className="empty">
            <span className="empty-icon">
              <Archive size={20} />
            </span>
            <strong>{t.backups.empty}</strong>
            <span>{t.backups.emptyHint}</span>
          </div>
        ) : (
          <ul className="rows">
            {backups.map((b) => (
              <li key={b.fileName} className="row">
                <span className="row-icon">
                  {b.scope === "full" ? <Archive size={15} /> : <DatabaseBackup size={15} />}
                </span>
                <span className="row-main">
                  <span className="row-title">
                    {formatDateTime(b.createdAt)}
                    <span className="subtle"> · {formatRelative(b.createdAt)}</span>
                  </span>
                  <span className="row-tags">
                    <span className="tag" data-tone={b.kind === "manual" ? "brand" : undefined}>
                      {t.backups.kinds[b.kind]}
                    </span>
                    <span className="tag">{t.backups.scopes[b.scope]}</span>
                    <span className="subtle">
                      {formatBytes(b.size)} · {t.common.minecraft(b.mcVersion)}
                    </span>
                  </span>
                </span>
                <span className="row-actions">
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() => restore(b)}
                    disabled={working || !stopped}
                    title={stopped ? undefined : t.backups.restoreNeedsStop}
                  >
                    <RotateCcw size={14} />
                    {t.backups.restore}
                  </button>
                  <button
                    className="btn btn-ghost btn-icon btn-sm"
                    onClick={() => remove(b)}
                    disabled={working}
                    title={t.common.delete}
                    aria-label={t.common.delete}
                  >
                    <Trash2 size={14} />
                  </button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
