import { Crown, LogOut, UserCheck, Users, X } from "lucide-react";
import { Avatar } from "./Avatar";
import { useT } from "../i18n";
import type { Rejection } from "../lib/useAppData";

interface Props {
  players: string[];
  live: boolean;
  rejections: Rejection[];
  onCommand: (command: string) => void;
  onLetIn: (name: string) => void;
  onDismiss: (name: string) => void;
}

export function PlayersPanel({ players, live, rejections, onCommand, onLetIn, onDismiss }: Props) {
  const t = useT();
  return (
    <section className="card players">
      <header className="card-header">
        <span className="card-title">
          <Users size={16} />
          {t.playersPanel.title}
        </span>
        <span className="count">{players.length}</span>
      </header>
      {players.length === 0 ? (
        <div className="empty">
          <span className="empty-icon">
            <Users size={20} />
          </span>
          <strong>{t.playersPanel.nobody}</strong>
          <span>{live ? t.playersPanel.share : t.playersPanel.startFirst}</span>
        </div>
      ) : (
        <ul className="player-list">
          {players.map((name) => (
            <li key={name} className="player">
              <Avatar name={name} />
              <span className="player-name">{name}</span>
              <div className="player-actions">
                <button
                  className="btn btn-ghost btn-icon btn-sm"
                  title={t.playersPanel.op}
                  aria-label={t.playersPanel.op}
                  onClick={() => onCommand(`op ${name}`)}
                >
                  <Crown size={14} />
                </button>
                <button
                  className="btn btn-ghost btn-icon btn-sm"
                  title={t.playersPanel.kick}
                  aria-label={t.playersPanel.kick}
                  onClick={() => onCommand(`kick ${name}`)}
                >
                  <LogOut size={14} />
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      {rejections.length > 0 && (
        <div className="attempts">
          <div className="attempts-title">{t.playersPanel.attempts}</div>
          {rejections.map((r) => (
            <div key={r.name} className="attempt">
              <Avatar name={r.name} size={26} />
              <div className="attempt-text">
                <span className="player-name">{r.name}</span>
                <span className="subtle">
                  {r.reason === "banned" ? t.playersPanel.banned : t.playersPanel.notWhitelisted}
                </span>
                {r.reason === "notWhitelisted" && (
                  <button className="btn btn-secondary btn-sm attempt-action" onClick={() => onLetIn(r.name)}>
                    <UserCheck size={14} />
                    {t.playersPanel.letIn}
                  </button>
                )}
              </div>
              <button
                className="btn btn-ghost btn-icon btn-sm"
                title={t.common.dismiss}
                aria-label={t.common.dismiss}
                onClick={() => onDismiss(r.name)}
              >
                <X size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
