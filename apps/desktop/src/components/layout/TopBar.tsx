import type { WorkspaceName, DatabankWorkspace } from '../../types';
import { formatNumber } from '../../view';

const workspaceTitles: Record<WorkspaceName, string> = {
  Home: "Overview",
  Databank: "Databank",
  Discover: "Discover",
  "Search Settings": "Settings",
  Vault: "Certified",
  "Data Lab": "Data Manager",
  "Parity Lab": "Retester",
  Portfolio: "Portfolio",
  Deploy: "Deployment",
};

export function TopBar({
  activeWorkspace,
  workspace,
  resultsDetailFp,
  resultsOrigin,
  onBackToList,
  onOpenDatabank,
  loading
}: {
  activeWorkspace: WorkspaceName;
  workspace: DatabankWorkspace | null;
  resultsDetailFp: string | null;
  resultsOrigin: WorkspaceName;
  onBackToList: () => void;
  onOpenDatabank: () => void;
  loading: boolean;
}) {
  return (
    <header className="topbar">
      <nav className="crumbs" aria-label="Breadcrumb">
        <span>Strategies</span>
        <i aria-hidden="true" />
        {resultsDetailFp ? (
          <>
            <button type="button" className="crumb-link" onClick={onBackToList}>
              {workspaceTitles[resultsOrigin] || resultsOrigin}
            </button>
            <i aria-hidden="true" />
            <strong>Results detail</strong>
          </>
        ) : (
          <strong>{workspaceTitles[activeWorkspace] || activeWorkspace}</strong>
        )}
      </nav>
      <div className="topbar-actions">
        {workspace && (
          <span className="topbar-chip" title={workspace.sourcePath}>
            {formatNumber(workspace.elites.length)} elites · {workspace.sourcePath.split("/").at(-1)}
          </span>
        )}
        {activeWorkspace === "Databank" && (
          <button className="primary" onClick={onOpenDatabank} disabled={loading}>
            {loading ? "Validating…" : workspace ? "Open another" : "Open databank"}
          </button>
        )}
        <div className="topbar-utilities" aria-label="Application status">
          <span className="topbar-icon" title="QuantForge notifications">
            <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M7.5 17h9l-1.2-1.8V11a3.3 3.3 0 0 0-6.6 0v4.2L7.5 17Zm3 2h3" /></svg>
          </span>
          <span className="topbar-icon" title="Research methodology">
            <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="7.5" /><path d="M10.4 9.6a1.8 1.8 0 0 1 3.4.8c0 1.6-1.8 1.7-1.8 3.1m0 2.6v.1" /></svg>
          </span>
          <span className="topbar-icon" title="Dark workspace">
            <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3.2" /><path d="M12 3.3v2M12 18.7v2M3.3 12h2M18.7 12h2M5.9 5.9l1.4 1.4m9.4 9.4 1.4 1.4m0-12.2-1.4 1.4M7.3 16.7l-1.4 1.4" /></svg>
          </span>
          <span className="topbar-divider" />
          <span className="user-avatar">DA</span>
          <span className="user-copy">
            <strong>Daniel</strong>
            <small>Local</small>
          </span>
          <span className="user-badge">Pro</span>
        </div>
      </div>
    </header>
  );
}
