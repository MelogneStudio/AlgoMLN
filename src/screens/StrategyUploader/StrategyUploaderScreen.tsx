import { useRef, useState, type DragEvent } from 'react';
import { Button } from '../../components/Button/Button';
import styles from './StrategyUploaderScreen.module.css';

interface StrategyUploaderScreenProps {
  open: boolean;
  onClose: () => void;
  onOpenEditor: () => void;
  onLoadSource: (source: string) => void;
}

export function StrategyUploaderScreen({
  open,
  onClose,
  onOpenEditor,
  onLoadSource,
}: StrategyUploaderScreenProps) {
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  if (!open) return null;

  const handleDragOver = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    setDragging(true);
  };

  const handleDragLeave = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    setDragging(false);
  };

  const readFile = async (file: File) => {
    if (!file.name.endsWith('.algomln')) {
      setError('Only .algomln files are supported.');
      return;
    }
    try {
      const text = await file.text();
      onLoadSource(text);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    setDragging(false);
    const file = e.dataTransfer.files?.[0];
    if (file) void readFile(file);
  };

  const handleChoose = () => {
    fileInputRef.current?.click();
  };

  const handleFileInput = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      await readFile(file);
    }
    // allow re-selecting the same file
    if (fileInputRef.current) fileInputRef.current.value = '';
  };

  const onDropZoneKey = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      handleChoose();
    }
  };

  return (
    <div className={styles.overlay} role="dialog" aria-modal="true">
      <div className={styles.card}>
        <h2 className={styles.title}>Upload your strategy</h2>

        <div
          className={`${styles.dropzone} ${dragging ? styles.dropzoneActive : ''}`}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          onDrop={handleDrop}
          onClick={handleChoose}
          onKeyDown={onDropZoneKey}
          role="button"
          tabIndex={0}
          aria-label="Drop a .algomln file here, or press Enter to browse"
        >
          <div className={styles.dropIcon} aria-hidden>
            <svg viewBox="0 0 64 64" width="56" height="56" fill="none">
              <path
                d="M16 8h24l12 12v36a4 4 0 0 1-4 4H16a4 4 0 0 1-4-4V12a4 4 0 0 1 4-4z"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinejoin="round"
              />
              <path
                d="M40 8v12h12"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinejoin="round"
              />
              <path
                d="M24 36l8-8 8 8M32 28v18"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </div>
          <span className={styles.dropText}>
            Drag and drop your <code>.algomln</code> strategy here
          </span>
          <span className={styles.dropHint}>or click to browse</span>
          <input
            ref={fileInputRef}
            type="file"
            accept=".algomln,text/plain"
            onChange={handleFileInput}
            style={{ display: 'none' }}
          />
        </div>

        <Button
          variant="ghost"
          onClick={handleChoose}
          icon={
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
              <path
                d="M12 4v12M6 10l6-6 6 6M4 20h16"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          }
        >
          Or choose a file
        </Button>

        {error && (
          <div className={styles.error} role="alert">
            {error}
          </div>
        )}

        <div className={styles.footer}>
          <span>Don&apos;t have a strategy file?</span>
          <Button
            variant="code"
            onClick={() => {
              onClose();
              onOpenEditor();
            }}
            icon={
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
                <path
                  d="M9 6l-6 6 6 6M15 6l6 6-6 6"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            }
          >
            Open Editor
          </Button>
        </div>
      </div>
    </div>
  );
}
