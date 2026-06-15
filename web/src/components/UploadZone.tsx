import { useState, useRef, useCallback, useEffect } from 'react';
import api from '../api/client';
import { Icon } from './Icon';
import { hasNasfilesDrag } from '../lib/fileDrag';
import { useGlobalDragCleanup } from '../lib/dragState';

interface UploadZoneProps {
  root: string;
  path: string;
  children: React.ReactNode;
  onUploadComplete: () => void;
  canUpload?: boolean;
}

interface UploadItem {
  file: File;
  progress: number;
  status: 'pending' | 'uploading' | 'done' | 'error';
  error?: string;
}

export function UploadZone({ root, path, children, onUploadComplete, canUpload = true }: UploadZoneProps) {
  const [isDragging, setIsDragging] = useState(false);
  const [uploads, setUploads] = useState<UploadItem[]>([]);
  const [showProgress, setShowProgress] = useState(false);
  const dragCounter = useRef(0);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const resetDragState = useCallback(() => {
    dragCounter.current = 0;
    setIsDragging(false);
  }, []);

  useGlobalDragCleanup(resetDragState);

  useEffect(() => {
    if (!canUpload) resetDragState();
  }, [canUpload, resetDragState]);

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    if (!canUpload) return;
    if (hasNasfilesDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current++;
    if (e.dataTransfer.types.includes('Files')) {
      setIsDragging(true);
    }
  }, [canUpload]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    if (!canUpload) return;
    if (hasNasfilesDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current = Math.max(0, dragCounter.current - 1);
    if (dragCounter.current === 0) {
      setIsDragging(false);
    }
  }, [canUpload]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (!canUpload) return;
    if (hasNasfilesDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
  }, [canUpload]);

  const uploadFiles = useCallback(async (files: File[]) => {
    if (files.length === 0) return;

    const items: UploadItem[] = files.map((f) => ({
      file: f,
      progress: 0,
      status: 'pending' as const,
    }));
    setUploads(items);
    setShowProgress(true);

    // Upload in batches of 3
    const batchSize = 3;
    for (let i = 0; i < files.length; i += batchSize) {
      const batch = files.slice(i, i + batchSize);
      await Promise.allSettled(
        batch.map(async (file, bIdx) => {
          const idx = i + bIdx;
          setUploads((prev) =>
            prev.map((u, j) => (j === idx ? { ...u, status: 'uploading' } : u))
          );
          try {
            await api.upload(root, path, [file], (pct) => {
              setUploads((prev) =>
                prev.map((u, j) => (j === idx ? { ...u, progress: pct } : u))
              );
            });
            setUploads((prev) =>
              prev.map((u, j) => (j === idx ? { ...u, status: 'done', progress: 100 } : u))
            );
          } catch (err) {
            setUploads((prev) =>
              prev.map((u, j) =>
                j === idx
                  ? { ...u, status: 'error', error: String(err) }
                  : u
              )
            );
          }
        })
      );
    }

    onUploadComplete();
    // Auto-hide progress after 2s
    setTimeout(() => {
      setShowProgress(false);
      setUploads([]);
    }, 2000);
  }, [root, path, onUploadComplete]);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      if (!canUpload) return;
      if (hasNasfilesDrag(e.dataTransfer)) return;
      e.preventDefault();
      e.stopPropagation();
      resetDragState();

      const files = Array.from(e.dataTransfer.files);
      await uploadFiles(files);
    },
    [canUpload, resetDragState, uploadFiles]
  );

  const handleFileSelect = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files || []);
      await uploadFiles(files);
      // Reset input so same file can be re-selected
      if (fileInputRef.current) fileInputRef.current.value = '';
    },
    [uploadFiles]
  );

  const totalFiles = uploads.length;
  const doneFiles = uploads.filter((u) => u.status === 'done').length;
  const overallProgress = totalFiles > 0
    ? Math.round(uploads.reduce((sum, u) => sum + u.progress, 0) / totalFiles)
    : 0;

  return (
    <div
      style={{ position: 'relative', flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      {/* Hidden file input for programmatic trigger */}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        style={{ display: 'none' }}
        onChange={handleFileSelect}
      />

      {/* Main content */}
      {children}

      {/* Drag overlay */}
      {isDragging && (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: 'rgba(59, 130, 246, 0.08)',
            border: '2px dashed var(--color-accent)',
            borderRadius: 'var(--radius-lg)',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 'var(--space-3)',
            zIndex: 20,
            backdropFilter: 'blur(2px)',
          }}
          className="fade-in"
        >
          <Icon name="folder" size={48} color="var(--color-accent)" style={{ opacity: 0.7 }} />
          <div style={{
            fontSize: 'var(--text-lg)',
            fontWeight: 600,
            color: 'var(--color-accent)',
          }}>
            Drop to upload
          </div>
          <div style={{
            fontSize: 'var(--text-sm)',
            color: 'var(--color-fg-muted)',
          }}>
            Files will be uploaded to the current folder
          </div>
        </div>
      )}

      {/* Upload progress panel */}
      {showProgress && (
        <div
          style={{
            position: 'absolute',
            bottom: 'var(--space-4)',
            right: 'var(--space-4)',
            width: 320,
            background: 'var(--color-bg)',
            border: '1px solid var(--color-border)',
            borderRadius: 'var(--radius-lg)',
            boxShadow: 'var(--shadow-lg)',
            padding: 'var(--space-4)',
            zIndex: 30,
          }}
          className="slide-in"
        >
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 'var(--space-3)',
          }}>
            <span style={{ fontWeight: 600, fontSize: 'var(--text-sm)' }}>
              Uploading {doneFiles}/{totalFiles}
            </span>
            <span style={{ fontSize: 'var(--text-sm)', color: 'var(--color-fg-muted)' }}>
              {overallProgress}%
            </span>
          </div>

          {/* Overall progress bar */}
          <div style={{
            height: 4,
            borderRadius: 2,
            background: 'var(--color-bg-muted)',
            overflow: 'hidden',
            marginBottom: 'var(--space-3)',
          }}>
            <div style={{
              height: '100%',
              width: `${overallProgress}%`,
              background: 'var(--color-accent)',
              borderRadius: 2,
              transition: 'width 200ms ease-out',
            }} />
          </div>

          {/* File list (max 4 visible) */}
          <div style={{ maxHeight: 140, overflowY: 'auto' }}>
            {uploads.slice(0, 10).map((item, i) => (
              <div
                key={i}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 'var(--space-2)',
                  padding: 'var(--space-1) 0',
                  fontSize: 'var(--text-xs)',
                }}
              >
                <span style={{
                  color: item.status === 'done'
                    ? 'var(--color-success)'
                    : item.status === 'error'
                      ? 'var(--color-danger)'
                      : 'var(--color-fg-muted)',
                }}>
                  {item.status === 'done' ? '✓' : item.status === 'error' ? '✗' : '⋯'}
                </span>
                <span style={{
                  flex: 1,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  color: 'var(--color-fg)',
                }}>
                  {item.file.name}
                </span>
                <span className="tabular-nums" style={{ color: 'var(--color-fg-subtle)' }}>
                  {item.progress}%
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );

  // Expose the trigger method via a callback
}

/** Hook to get a ref to trigger file upload programmatically */
export function useUploadTrigger() {
  const inputRef = useRef<HTMLInputElement>(null);
  const trigger = useCallback(() => {
    inputRef.current?.click();
  }, []);
  return { inputRef, trigger };
}
