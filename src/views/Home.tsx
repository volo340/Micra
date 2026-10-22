import { Activity, ArrowRight, Coffee, Globe2, History, PackageSearch, ShieldCheck } from "lucide-react";
import { Logo } from "../components/Logo";
import { useT } from "../i18n";

const ICONS = [PackageSearch, Coffee, Activity, History, Globe2, ShieldCheck];

export function Home({ onCreate, hasServers }: { onCreate: () => void; hasServers: boolean }) {
  const t = useT();
  return (
    <div className="page home">
      <section className="hero">
        <div className="hero-logo">
          <Logo size={112} />
        </div>
        <h1>{t.home.title}</h1>
        <p className="hero-text">{t.home.text}</p>
        <button className="btn btn-primary btn-lg" onClick={onCreate}>
          {hasServers ? t.home.createAnother : t.home.createFirst}
          <ArrowRight size={17} />
        </button>
      </section>

      <div className="features">
        {t.home.features.map(({ title, text }, i) => {
          const Icon = ICONS[i];
          return (
            <div key={title} className="card feature">
              <span className="feature-icon">
                <Icon size={18} />
              </span>
              <h3>{title}</h3>
              <p className="muted">{text}</p>
            </div>
          );
        })}
      </div>
    </div>
  );
}
