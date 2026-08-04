import type { WorkspaceName } from '../../types';

type NavIcon = 'overview' | 'discover' | 'databank' | 'retester' | 'data' | 'portfolio' | 'settings';

const NAV_GLYPHS: Record<NavIcon, string> = {
  overview: 'M4 11.5 12 4.5l8 7M6 10.5V19h12v-8.5',
  discover: 'M10.5 4.5a6 6 0 1 0 0 12 6 6 0 0 0 0-12ZM15 15l4.5 4.5',
  databank: 'M4.5 6.5h15M4.5 12h15M4.5 17.5h15M9 4v16',
  retester: 'M5 19V9m4.7 10V5m4.6 14v-7m4.7 7V8',
  data: 'M12 4c4 0 7 1.2 7 2.7S16 9.4 12 9.4 5 8.2 5 6.7 8 4 12 4Zm7 5v8.3c0 1.5-3 2.7-7 2.7s-7-1.2-7-2.7V9',
  portfolio: 'M4.5 8.5h15v11h-15zM9 8.5V6a3 3 0 0 1 6 0v2.5',
  settings: 'M12 9.2a2.8 2.8 0 1 0 0 5.6 2.8 2.8 0 0 0 0-5.6ZM12 3.5v2.2M12 18.3v2.2M5.5 12H3.3m17.4 0h-2.2M7.4 7.4 5.9 5.9m12.2 12.2-1.5-1.5M16.6 7.4l1.5-1.5M5.9 18.1l1.5-1.5',
};

const navigation: { name: WorkspaceName; label: string; icon: NavIcon }[] = [
  { name: 'Home', label: 'Overview', icon: 'overview' },
  { name: 'Discover', label: 'Discover', icon: 'discover' },
  { name: 'Databank', label: 'Databank', icon: 'databank' },
  { name: 'Parity Lab', label: 'Retester', icon: 'retester' },
  { name: 'Data Lab', label: 'Data Manager', icon: 'data' },
  { name: 'Portfolio', label: 'Portfolio', icon: 'portfolio' },
  { name: 'Search Settings', label: 'Settings', icon: 'settings' },
];

function NavGlyph({ icon }: { icon: NavIcon }) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" className="nav-glyph">
      <path d={NAV_GLYPHS[icon]} />
    </svg>
  );
}

export function NavigationRail({ activeWorkspace, onNavigate }: { activeWorkspace: WorkspaceName; onNavigate: (ws: WorkspaceName) => void }) {
  return (
    <aside className="rail" aria-label="QuantForge">
      <div className="rail-brand" title="QuantForge">
        <span className="rail-monogram" aria-hidden="true">QF</span>
        <span className="rail-brand-label">QuantForge</span>
      </div>
      <nav className="rail-nav" aria-label="Workspaces">
        {navigation.map(({ name, label, icon }) => (
          <button
            aria-current={name === activeWorkspace ? "page" : undefined}
            className={name === activeWorkspace ? "rail-item active" : "rail-item"}
            key={name}
            onClick={() => onNavigate(name)}
            title={label}
            type="button"
          >
            <NavGlyph icon={icon} />
            <span>{label}</span>
          </button>
        ))}
      </nav>
      <div className="rail-foot" title="Rust engine runs locally">
        <span className="status-light" />
        <span>Local</span>
      </div>
    </aside>
  );
}
