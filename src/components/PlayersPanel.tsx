import { Crown, LogOut, Users } from "lucide-react";
import { Avatar } from "./Avatar";

interface Props {
  players: string[];
  live: boolean;
  onCommand: (command: string) => void;
}

export function PlayersPanel({ players, live, onCommand }: Props) {
  return (
    <section className="card players">
      <header className="card-header">
        <span className="card-title">
          <Users size={16} />
          Jugadores
        </span>
        <span className="count">{players.length}</span>
      </header>
      {players.length === 0 ? (
        <div className="empty">
          <span className="empty-icon">
            <Users size={20} />
          </span>
          <strong>Nadie conectado</strong>
          <span>
            {live ? "Comparte la dirección de arriba con tus amigos." : "Enciende el servidor para que entren."}
          </span>
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
                  title="Dar permisos de operador"
                  onClick={() => onCommand(`op ${name}`)}
                >
                  <Crown size={14} />
                </button>
                <button
                  className="btn btn-ghost btn-icon btn-sm"
                  title="Expulsar del servidor"
                  onClick={() => onCommand(`kick ${name}`)}
                >
                  <LogOut size={14} />
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
