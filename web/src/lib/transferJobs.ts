import type { TransferJob } from '../api/client';

export function isActiveTransferJob(job: TransferJob) {
  return job.status === 'queued' || job.status === 'running';
}

export function transferJobsForTarget(jobs: TransferJob[], root: string, path: string) {
  return jobs.filter(
    (job) =>
      isActiveTransferJob(job) &&
      (job.operation === 'copy' || job.operation === 'move') &&
      job.dest_root === root &&
      job.dest_path === path,
  );
}

export function moveJobsForSourcePath(jobs: TransferJob[], root: string, path: string) {
  return jobs.filter(
    (job) =>
      isActiveTransferJob(job) &&
      job.operation === 'move' &&
      job.source_root === root &&
      job.paths.includes(path),
  );
}

export function transferProgressPercent(jobs: TransferJob[]) {
  const totalBytes = jobs.reduce((sum, job) => sum + job.total_bytes, 0);
  const transferredBytes = jobs.reduce((sum, job) => sum + job.transferred_bytes, 0);
  if (totalBytes > 0) {
    return Math.min(100, Math.round((transferredBytes / totalBytes) * 100));
  }

  const totalEntries = jobs.reduce((sum, job) => sum + job.total_entries, 0);
  const completedEntries = jobs.reduce((sum, job) => sum + job.completed_entries, 0);
  if (totalEntries > 0) {
    return Math.min(100, Math.round((completedEntries / totalEntries) * 100));
  }

  return 0;
}

export interface TransferPlaceholder {
  key: string;
  name: string;
  job: TransferJob;
}

export function incomingTransferPlaceholders(
  jobs: TransferJob[],
  root: string,
  path: string,
  existingNames: string[] = [],
) {
  const existing = new Set(existingNames);
  return jobs
    .filter(
      (job) =>
        isActiveTransferJob(job) &&
        (job.operation === 'copy' || job.operation === 'move') &&
        job.dest_root === root &&
        job.dest_path === path,
    )
    .flatMap((job) =>
      job.paths
        .map((sourcePath) => {
          const name = basename(sourcePath);
          if (!name || existing.has(name)) return null;
          return { key: `${job.id}:${sourcePath}`, name, job };
        })
        .filter((placeholder): placeholder is TransferPlaceholder => placeholder !== null),
    );
}

function basename(path: string) {
  const parts = path.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
