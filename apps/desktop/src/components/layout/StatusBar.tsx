import { formatNumber } from '../../view';

export function StatusBar({
  engineStatus,
  taskCount,
  evalsPerHour
}: {
  engineStatus: 'online' | 'offline' | 'busy';
  taskCount: number;
  evalsPerHour: number;
}) {
  return (
    <footer className="status-bar">
      <div className="status-bar-left">
        <span className={`status-indicator status-${engineStatus}`} />
        <span className="status-text">
          Engine: {engineStatus.charAt(0).toUpperCase() + engineStatus.slice(1)}
        </span>
      </div>
      <div className="status-bar-right">
        {taskCount > 0 && (
          <span className="status-metric">
            {taskCount} active task{taskCount !== 1 ? 's' : ''}
          </span>
        )}
        {evalsPerHour > 0 && (
          <span className="status-metric">
            {formatNumber(evalsPerHour)} evals/h
          </span>
        )}
        <span className="status-version">v1.0.0</span>
      </div>
    </footer>
  );
}
