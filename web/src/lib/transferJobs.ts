import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import api from "../api/client";
import type { TransferJob } from "../api/client";

export function isActiveTransferJob(job: TransferJob) {
  return job.status === "queued" || job.status === "running";
}

/** Poll cadence while something is actually moving. */
const ACTIVE_POLL_MS = 1000;
/** Poll cadence while the queue is empty — just enough to notice a job that
 *  another tab or the share-target flow started. */
const IDLE_POLL_MS = 30_000;

function isInterestingTransferJob(job: TransferJob) {
  return isActiveTransferJob(job) || job.status === "paused_needs_confirmation";
}

/**
 * The shared transfer-jobs poll.
 *
 * There is no websocket, so progress arrives by polling. The interval adapts:
 * one second while a job is in flight, but backing off to every 30s when the
 * queue is empty. The old fixed 1Hz poll ran for the entire lifetime of every
 * logged-in session, so an idle browser tab generated 86k requests a day — each
 * one a round trip the server had to answer while doing real filesystem work.
 *
 * Both `TopBar` and the file browser call this. They share one React Query key,
 * so this is a single request regardless of how many components subscribe, and
 * `jobs` keeps a stable identity between polls that return the same data.
 */
export function useTransferJobs(enabled: boolean) {
  const { data } = useQuery({
    queryKey: ["transfer-jobs"],
    queryFn: api.transferJobs,
    enabled,
    staleTime: ACTIVE_POLL_MS,
    refetchIntervalInBackground: false,
    refetchInterval: (query) => {
      if (!enabled) return false;
      const jobs = query.state.data?.jobs ?? [];
      return jobs.some(isInterestingTransferJob) ? ACTIVE_POLL_MS : IDLE_POLL_MS;
    },
  });

  return useMemo(() => data?.jobs ?? EMPTY_JOBS, [data?.jobs]);
}

/** Shared empty array so "no jobs" is a stable reference across renders. */
const EMPTY_JOBS: TransferJob[] = [];

export function transferJobsForTarget(
  jobs: TransferJob[],
  root: string,
  path: string,
) {
  return jobs.filter(
    (job) =>
      isActiveTransferJob(job) &&
      (job.operation === "copy" || job.operation === "move") &&
      job.dest_root === root &&
      job.dest_path === path,
  );
}

export function moveJobsForSourcePath(
  jobs: TransferJob[],
  root: string,
  path: string,
) {
  return jobs.filter(
    (job) =>
      isActiveTransferJob(job) &&
      job.operation === "move" &&
      job.source_root === root &&
      job.paths.includes(path),
  );
}

export function transferProgressPercent(jobs: TransferJob[]) {
  const totalBytes = jobs.reduce((sum, job) => sum + job.total_bytes, 0);
  const transferredBytes = jobs.reduce(
    (sum, job) => sum + job.transferred_bytes,
    0,
  );
  if (totalBytes > 0) {
    return Math.min(100, Math.round((transferredBytes / totalBytes) * 100));
  }

  const totalEntries = jobs.reduce((sum, job) => sum + job.total_entries, 0);
  const completedEntries = jobs.reduce(
    (sum, job) => sum + job.completed_entries,
    0,
  );
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
        (job.operation === "copy" || job.operation === "move") &&
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
        .filter(
          (placeholder): placeholder is TransferPlaceholder =>
            placeholder !== null,
        ),
    );
}

function basename(path: string) {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
