import { useCallback, useEffect, useRef, useState } from 'react';

export interface SplitPaneProps {
  direction: 'horizontal' | 'vertical';
  defaultSize?: number;
  minSize?: number;
  maxSize?: number;
  storageKey?: string;
  children: [React.ReactNode, React.ReactNode];
  className?: string;
  onResize?: (size: number) => void;
}

export function SplitPane({
  direction,
  defaultSize = 300,
  minSize = 100,
  maxSize,
  storageKey,
  children,
  className = '',
  onResize
}: SplitPaneProps) {
  const getInitialSize = () => {
    if (storageKey) {
      const stored = localStorage.getItem(storageKey);
      if (stored) {
        const parsed = parseInt(stored, 10);
        if (!isNaN(parsed)) return parsed;
      }
    }
    return defaultSize;
  };

  const [size, setSize] = useState(getInitialSize);
  const [isDragging, setIsDragging] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
  }, []);

  const handleMouseMove = useCallback(
    (e: MouseEvent) => {
      if (!isDragging || !containerRef.current) return;

      const containerRect = containerRef.current.getBoundingClientRect();
      let newSize;

      if (direction === 'horizontal') {
        newSize = e.clientX - containerRect.left;
      } else {
        newSize = e.clientY - containerRect.top;
      }

      if (minSize !== undefined && newSize < minSize) newSize = minSize;
      if (maxSize !== undefined && newSize > maxSize) newSize = maxSize;

      setSize(newSize);
      onResize?.(newSize);
    },
    [isDragging, direction, minSize, maxSize, onResize]
  );

  const handleMouseUp = useCallback(() => {
    if (isDragging) {
      setIsDragging(false);
      if (storageKey) {
        localStorage.setItem(storageKey, size.toString());
      }
    }
  }, [isDragging, storageKey, size]);

  useEffect(() => {
    if (isDragging) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = direction === 'horizontal' ? 'col-resize' : 'row-resize';
      document.body.style.userSelect = 'none';
    } else {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    }

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
  }, [isDragging, handleMouseMove, handleMouseUp, direction]);

  const isHorizontal = direction === 'horizontal';

  return (
    <div
      ref={containerRef}
      className={`split-pane split-pane-${direction} ${className}`}
      style={{
        display: 'flex',
        flexDirection: isHorizontal ? 'row' : 'column',
        width: '100%',
        height: '100%',
        overflow: 'hidden'
      }}
    >
      <div
        className="split-pane-panel"
        style={{
          [isHorizontal ? 'width' : 'height']: size,
          flexShrink: 0,
          overflow: 'auto'
        }}
      >
        {children[0]}
      </div>
      <div
        className="split-pane-divider"
        onMouseDown={handleMouseDown}
        style={{
          cursor: isHorizontal ? 'col-resize' : 'row-resize',
          [isHorizontal ? 'width' : 'height']: '4px',
          backgroundColor: 'transparent',
          zIndex: 10
        }}
      />
      <div
        className="split-pane-panel"
        style={{
          flex: 1,
          overflow: 'auto'
        }}
      >
        {children[1]}
      </div>
    </div>
  );
}
