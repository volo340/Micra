import { Activity, ArrowRight, Coffee, PackageSearch } from "lucide-react";
import { Logo } from "../components/Logo";

const FEATURES = [
  {
    icon: PackageSearch,
    title: "Modpacks en un clic",
    text: "Busca en Modrinth y CurseForge, o crea un servidor Vanilla o Fabric.",
  },
  {
    icon: Coffee,
    title: "Java se instala solo",
    text: "Micra descarga la versión exacta que necesita cada servidor, sin tocar tu sistema.",
  },
  {
    icon: Activity,
    title: "Todo a la vista",
    text: "Consola, CPU, memoria y jugadores conectados en tiempo real.",
  },
];

export function Home({ onCreate, hasServers }: { onCreate: () => void; hasServers: boolean }) {
  return (
    <div className="page home">
      <section className="hero">
        <div className="hero-logo">
          <Logo size={112} />
        </div>
        <h1>Convierte tu PC en un servidor de Minecraft</h1>
        <p className="hero-text">
          Elige qué quieres jugar, decide cuánta memoria darle y pulsa encender. Micra se encarga del resto.
        </p>
        <button className="btn btn-primary btn-lg" onClick={onCreate}>
          {hasServers ? "Crear otro servidor" : "Crear mi primer servidor"}
          <ArrowRight size={17} />
        </button>
      </section>

      <div className="features">
        {FEATURES.map(({ icon: Icon, title, text }) => (
          <div key={title} className="card feature">
            <span className="feature-icon">
              <Icon size={18} />
            </span>
            <h3>{title}</h3>
            <p className="muted">{text}</p>
          </div>
        ))}
      </div>
    </div>
  );
}
