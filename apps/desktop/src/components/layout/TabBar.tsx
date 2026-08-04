import type { WorkspaceName } from '../../types';

type ModuleTab = {
  id: string;
  label: string;
  workspaces: WorkspaceName[];
};

const MODULES: ModuleTab[] = [
  { id: 'builder', label: 'Builder', workspaces: ['Home', 'Discover', 'Search Settings'] },
  { id: 'databank', label: 'Databank', workspaces: ['Databank', 'Vault'] },
  { id: 'retester', label: 'Retester', workspaces: ['Parity Lab'] },
  { id: 'data', label: 'Data Manager', workspaces: ['Data Lab'] },
  { id: 'portfolio', label: 'Portfolio', workspaces: ['Portfolio', 'Deploy'] },
];

export function TabBar({ activeWorkspace, onNavigate }: { activeWorkspace: WorkspaceName; onNavigate: (ws: WorkspaceName) => void; }) {
  const activeModule = MODULES.find(m => m.workspaces.includes(activeWorkspace));

  return (
    <div className="module-tabs">
      {MODULES.map(module => (
        <button
          key={module.id}
          className={`module-tab ${activeModule?.id === module.id ? 'active' : ''}`}
          onClick={() => {
            if (activeModule?.id !== module.id) {
              onNavigate(module.workspaces[0]);
            }
          }}
        >
          {module.label}
        </button>
      ))}
    </div>
  );
}
