import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import videojs from 'video.js';
import '@videojs/http-streaming';
import type { FileEntry, PreviewStatus } from '../api/client';
import { Icon } from './Icon';

type MediaKind = 'video' | 'audio';

const PREVIEW_VIDEO_CONTENT_TYPE = 'application/x-mpegurl';
const PREVIEW_AUDIO_CONTENT_TYPE = 'audio/mp4; codecs="mp4a.40.2"';

type PlayerTimelineState = 'unknown' | 'live' | 'duration';

interface MediaPreviewProps {
  entry: FileEntry;
  kind: MediaKind;
  actualUrl: string;
  coverArtUrl?: string | null;
  canTranscode: boolean;
  createPreviewUrl: (session: string) => string;
  loadPreviewStatus: (session: string) => Promise<PreviewStatus>;
  loadFileInfo?: () => Promise<FileEntry & { path: string }>;
  initialFileInfo?: FileEntry | null;
  onInfoLoaded?: (info: FileEntry) => void;
}

type MediaDecision =
  | {
      mode: 'actual' | 'preview';
      sourceUrl: string;
      contentType: string;
      label: string;
      reason: string;
      debug: string[];
    }
  | {
      mode: 'blocked';
      label: string;
      reason: string;
      debug: string[];
    };

export function MediaPreview({
  entry,
  kind,
  actualUrl,
  coverArtUrl,
  canTranscode,
  createPreviewUrl,
  loadPreviewStatus,
  loadFileInfo,
  initialFileInfo,
  onInfoLoaded,
}: MediaPreviewProps) {
  const [loadedInfo, setLoadedInfo] = useState<FileEntry | null>(initialFileInfo ?? null);
  const [infoAttempted, setInfoAttempted] = useState(!loadFileInfo || Boolean(initialFileInfo));
  const [session, setSession] = useState(() => createPreviewSession());
  const [fallbackReason, setFallbackReason] = useState<string | null>(null);
  const [playerError, setPlayerError] = useState<string | null>(null);
  const [playerErrorMode, setPlayerErrorMode] = useState<MediaDecision['mode'] | null>(null);
  const [status, setStatus] = useState<PreviewStatus | null>(null);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [timelineState, setTimelineState] = useState<PlayerTimelineState>('unknown');

  useEffect(() => {
    setLoadedInfo(initialFileInfo ?? null);
    setInfoAttempted(!loadFileInfo || Boolean(initialFileInfo));
    setSession(createPreviewSession());
    setFallbackReason(null);
    setPlayerError(null);
    setPlayerErrorMode(null);
    setStatus(null);
    setDiagnosticsOpen(false);
    setTimelineState('unknown');
  }, [actualUrl, initialFileInfo, loadFileInfo]);

  useEffect(() => {
    if (initialFileInfo) {
      setLoadedInfo(initialFileInfo);
      setInfoAttempted(true);
    }
  }, [initialFileInfo]);

  useEffect(() => {
    if (!loadFileInfo || infoAttempted || loadedInfo) return;

    let cancelled = false;
    loadFileInfo()
      .then((info) => {
        if (cancelled) return;
        setLoadedInfo(info);
        setInfoAttempted(true);
        onInfoLoaded?.(info);
      })
      .catch(() => {
        if (!cancelled) setInfoAttempted(true);
      });

    return () => {
      cancelled = true;
    };
  }, [infoAttempted, loadFileInfo, loadedInfo, onInfoLoaded]);

  const effectiveEntry = loadedInfo ?? entry;
  const mediaInfo = effectiveEntry.media_info ?? null;
  const previewUrl = useMemo(() => createPreviewUrl(session), [createPreviewUrl, session]);
  const decision = useMemo(
    () =>
      chooseMediaSource({
        entry: effectiveEntry,
        kind,
        actualUrl,
        previewUrl,
        canTranscode,
        fallbackReason,
      }),
    [actualUrl, canTranscode, effectiveEntry, fallbackReason, kind, previewUrl],
  );

  useEffect(() => {
    if (decision.mode !== 'preview') return;

    let cancelled = false;
    let timer: number | undefined;

    const poll = () => {
      loadPreviewStatus(session)
        .then((nextStatus) => {
          if (cancelled) return;
          setStatus(nextStatus);
          if (nextStatus.state !== 'completed' && nextStatus.state !== 'failed') {
            timer = window.setTimeout(poll, 1000);
          }
        })
        .catch(() => {
          if (!cancelled) timer = window.setTimeout(poll, 1200);
        });
    };

    timer = window.setTimeout(poll, 700);

    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [decision.mode, loadPreviewStatus, session]);

  const handlePlayerError = useCallback(
    (message: string) => {
      setPlayerError(message);
      setPlayerErrorMode(decision.mode);
      if (decision.mode === 'actual' && canTranscode) {
        setSession(createPreviewSession());
        setStatus(null);
        setFallbackReason(`Direct playback failed: ${message}`);
      }
    },
    [canTranscode, decision.mode],
  );

  if (!infoAttempted) {
    return (
      <div data-preview-no-close style={panelStyle}>
        <div className="shimmer" style={{ width: 420, maxWidth: '80vw', height: 220, borderRadius: 8 }} />
        <div style={mutedTextStyle}>Preparing media...</div>
      </div>
    );
  }

  const currentSourceFailed =
    Boolean(playerError && playerErrorMode === decision.mode) || status?.state === 'failed' || decision.mode === 'blocked';
  const showDiagnostics = currentSourceFailed || diagnosticsOpen;

  if (decision.mode === 'blocked') {
    return (
      <div data-preview-no-close style={panelStyle}>
        <Icon name={kind === 'audio' ? 'music' : 'video'} size={56} color="rgba(255,255,255,0.55)" />
        <div style={titleStyle}>{effectiveEntry.name}</div>
        <MediaBadge label={decision.label} tone="error" />
        <div style={mutedTextStyle}>{decision.reason}</div>
        {showDiagnostics && (
          <Diagnostics
            session={session}
            decision={decision}
            status={status}
            playerError={playerError}
            mediaInfo={mediaInfo}
          />
        )}
      </div>
    );
  }

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--space-3)',
      }}
    >
      <div
        data-preview-no-close
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 'var(--space-2)',
          color: '#fff',
          flexWrap: 'wrap',
          textAlign: 'center',
        }}
      >
        <MediaBadge
          label={decision.label}
          tone={decision.mode === 'actual' ? 'ok' : 'preview'}
          onClick={decision.mode === 'preview' ? () => setDiagnosticsOpen((open) => !open) : undefined}
          pressed={decision.mode === 'preview' ? diagnosticsOpen : undefined}
        />
      </div>

      <div
        data-preview-no-close
        style={{
          width: kind === 'audio' ? 'min(560px, 92vw)' : 'min(100%, 1100px)',
          maxHeight: kind === 'video' ? 'calc(100vh - 190px)' : undefined,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 'var(--space-4)',
        }}
      >
        {kind === 'audio' && (
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)' }}>
            <AudioCoverArt url={coverArtUrl} name={effectiveEntry.name} />
            <div style={titleStyle}>{effectiveEntry.name}</div>
          </div>
        )}
        <VideoJsPlayer
          key={`${decision.mode}:${decision.sourceUrl}`}
          kind={kind}
          sourceUrl={decision.sourceUrl}
          contentType={decision.contentType}
          onError={handlePlayerError}
          onTimelineStateChange={setTimelineState}
        />
        {kind === 'video' && decision.mode === 'preview' && timelineState !== 'duration' && !currentSourceFailed && (
          <div data-preview-no-close style={timelineHintStyle}>
            Preparing timeline...
          </div>
        )}
      </div>

      {showDiagnostics && (
        <Diagnostics
          session={session}
          decision={decision}
          status={status}
          playerError={playerError}
          mediaInfo={mediaInfo}
        />
      )}
    </div>
  );
}

function AudioCoverArt({ url, name }: { url?: string | null; name: string }) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [url]);

  if (!url || failed) {
    return <Icon name="music" size={56} color="rgba(255,255,255,0.55)" />;
  }

  return (
    <img
      src={url}
      alt=""
      title={name}
      onError={() => setFailed(true)}
      style={{
        width: 220,
        maxWidth: '52vw',
        aspectRatio: '1 / 1',
        objectFit: 'cover',
        borderRadius: 'var(--radius-lg)',
        boxShadow: '0 20px 60px rgba(0,0,0,0.45)',
        background: 'rgba(255,255,255,0.08)',
      }}
    />
  );
}

function VideoJsPlayer({
  kind,
  sourceUrl,
  contentType,
  onError,
  onTimelineStateChange,
}: {
  kind: MediaKind;
  sourceUrl: string;
  contentType: string;
  onError: (message: string) => void;
  onTimelineStateChange?: (state: PlayerTimelineState) => void;
}) {
  const mediaRef = useRef<HTMLVideoElement | HTMLAudioElement | null>(null);
  const playerRef = useRef<ReturnType<typeof videojs> | null>(null);
  const onErrorRef = useRef(onError);
  const onTimelineStateChangeRef = useRef(onTimelineStateChange);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  useEffect(() => {
    onTimelineStateChangeRef.current = onTimelineStateChange;
  }, [onTimelineStateChange]);

  useEffect(() => {
    const mediaEl = mediaRef.current;
    if (!mediaEl) return;

    const player = videojs(mediaEl, {
      autoplay: true,
      controls: true,
      fluid: kind === 'video',
      responsive: true,
      preload: 'auto',
      sources: [{ src: sourceUrl, type: contentType }],
    });
    playerRef.current = player;

    const updateTimelineState = () => {
      if (kind !== 'video') {
        onTimelineStateChangeRef.current?.('duration');
        return;
      }

      const duration = player.duration();
      const durationValue = typeof duration === 'number' ? duration : Number.NaN;
      const seekable = player.seekable();
      const hasSeekableRange = seekable.length > 0 && Number.isFinite(seekable.end(seekable.length - 1));
      const liveTracker = (player as unknown as { liveTracker?: { isLive?: () => boolean } }).liveTracker;

      if (Number.isFinite(durationValue) && durationValue > 0) {
        onTimelineStateChangeRef.current?.('duration');
      } else if (liveTracker?.isLive?.() || durationValue === Infinity || hasSeekableRange) {
        onTimelineStateChangeRef.current?.('live');
      } else {
        onTimelineStateChangeRef.current?.('unknown');
      }
    };

    updateTimelineState();
    player.on('loadedmetadata', updateTimelineState);
    player.on('durationchange', updateTimelineState);
    player.on('timeupdate', updateTimelineState);
    player.on('loadedplaylist', updateTimelineState);

    player.on('error', () => {
      const error = player.error();
      onErrorRef.current(error?.message || 'media playback failed');
    });

    return () => {
      player.dispose();
      playerRef.current = null;
    };
  }, [contentType, kind, sourceUrl]);

  if (kind === 'audio') {
    return (
      <div data-preview-no-close className="nasfiles-media-player" data-vjs-player style={{ width: '100%' }}>
        <audio
          ref={mediaRef as React.RefObject<HTMLAudioElement>}
          className="video-js vjs-default-skin"
          playsInline
          style={{ width: '100%' }}
        />
      </div>
    );
  }

  return (
    <div
      data-preview-no-close
      className="nasfiles-media-player"
      data-vjs-player
      style={{
        width: '100%',
        maxHeight: 'calc(100vh - 190px)',
        boxShadow: '0 20px 60px rgba(0,0,0,0.5)',
        borderRadius: 'var(--radius-lg)',
        overflow: 'hidden',
        background: '#000',
      }}
    >
      <video
        ref={mediaRef as React.RefObject<HTMLVideoElement>}
        className="video-js vjs-default-skin vjs-big-play-centered"
        playsInline
        style={{ width: '100%', maxHeight: 'calc(100vh - 190px)' }}
      />
    </div>
  );
}

function chooseMediaSource({
  entry,
  kind,
  actualUrl,
  previewUrl,
  canTranscode,
  fallbackReason,
}: {
  entry: FileEntry;
  kind: MediaKind;
  actualUrl: string;
  previewUrl: string;
  canTranscode: boolean;
  fallbackReason: string | null;
}): MediaDecision {
  const mediaInfo = entry.media_info ?? null;
  const sourceType = sourceContentType(entry, kind, false);
  const sourceTypeWithCodecs = sourceContentType(entry, kind, true);
  const nativeSupport = nativeMediaSupport(entry, kind, sourceType, sourceTypeWithCodecs);
  const bitrate = mediaInfo?.bitrate_bps ?? estimateBitrate(entry);
  const debug = [
    `source type: ${sourceTypeWithCodecs || sourceType || 'unknown'}`,
    `native support: ${nativeSupport ? 'yes' : 'no'}`,
    bitrate ? `bitrate: ${formatBitrate(bitrate)}` : 'bitrate: unknown',
  ];

  if (fallbackReason) {
    if (canTranscode) {
      return {
        mode: 'preview',
        sourceUrl: previewUrl,
        contentType: previewContentType(kind),
        label: 'Preview',
        reason: fallbackReason,
        debug,
      };
    }

    return {
      mode: 'blocked',
      label: 'Playback problem',
      reason: `${fallbackReason}. Transcoding is unavailable.`,
      debug,
    };
  }

  if (kind === 'audio') {
    if (nativeSupport) {
      return {
        mode: 'actual',
        sourceUrl: actualUrl,
        contentType: sourceTypeWithCodecs || sourceType || 'audio/*',
        label: 'Actual media',
        reason: 'Browser supports this audio source.',
        debug,
      };
    }

    if (canTranscode) {
      return {
        mode: 'preview',
        sourceUrl: previewUrl,
        contentType: PREVIEW_AUDIO_CONTENT_TYPE,
        label: 'Preview',
        reason: 'Browser cannot play this audio source directly.',
        debug,
      };
    }

    return {
      mode: 'blocked',
      label: 'No preview',
      reason: 'Browser cannot play this audio source and transcoding is unavailable.',
      debug,
    };
  }

  const limit = videoBitrateLimit();
  debug.push(`video limit: ${formatBitrate(limit)}`);

  if (!nativeSupport) {
    if (canTranscode) {
      return {
        mode: 'preview',
        sourceUrl: previewUrl,
        contentType: PREVIEW_VIDEO_CONTENT_TYPE,
        label: 'Preview',
        reason: 'Browser cannot play this video source directly.',
        debug,
      };
    }

    return {
      mode: 'blocked',
      label: 'No preview',
      reason: 'Browser cannot play this video source and transcoding is unavailable.',
      debug,
    };
  }

  if (!bitrate) {
    if (canTranscode) {
      return {
        mode: 'preview',
        sourceUrl: previewUrl,
        contentType: PREVIEW_VIDEO_CONTENT_TYPE,
        label: 'Preview',
        reason: 'Source bitrate is unknown.',
        debug,
      };
    }

    return {
      mode: 'actual',
      sourceUrl: actualUrl,
      contentType: sourceTypeWithCodecs || sourceType || 'video/*',
      label: 'Actual media',
      reason: 'Transcoding is unavailable, so the original source is used.',
      debug,
    };
  }

  if (bitrate > limit) {
    if (canTranscode) {
      return {
        mode: 'preview',
        sourceUrl: previewUrl,
        contentType: PREVIEW_VIDEO_CONTENT_TYPE,
        label: 'Preview',
        reason: `Source bitrate ${formatBitrate(bitrate)} exceeds the current ${formatBitrate(limit)} limit.`,
        debug,
      };
    }

    return {
      mode: 'actual',
      sourceUrl: actualUrl,
      contentType: sourceTypeWithCodecs || sourceType || 'video/*',
      label: 'Actual media',
      reason: 'Transcoding is unavailable, so the high-bitrate source is used.',
      debug,
    };
  }

  return {
    mode: 'actual',
    sourceUrl: actualUrl,
    contentType: sourceTypeWithCodecs || sourceType || 'video/*',
    label: 'Actual media',
    reason: `Browser supports this source and ${formatBitrate(bitrate)} is within the ${formatBitrate(limit)} limit.`,
    debug,
  };
}

function previewContentType(kind: MediaKind): string {
  return kind === 'audio' ? PREVIEW_AUDIO_CONTENT_TYPE : PREVIEW_VIDEO_CONTENT_TYPE;
}

function nativeMediaSupport(
  entry: FileEntry,
  kind: MediaKind,
  sourceType: string,
  sourceTypeWithCodecs: string,
): boolean {
  if (canPlay(kind, sourceTypeWithCodecs)) return true;

  if (kind === 'audio') {
    return canPlay(kind, sourceType);
  }

  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  const browserFriendlyContainer = ext === 'mp4' || ext === 'm4v' || ext === 'webm' || ext === 'mov';
  return browserFriendlyContainer && canPlay(kind, sourceType);
}

function sourceContentType(entry: FileEntry, kind: MediaKind, includeCodecs: boolean): string {
  const base = entry.mime_type || fallbackMimeType(entry.name, kind);
  if (!base || !includeCodecs) return base;

  const mediaInfo = entry.media_info;
  const codecs = [mediaInfo?.video_mime_codec, mediaInfo?.audio_mime_codec].filter(Boolean);
  if (codecs.length === 0) return base;

  return `${base}; codecs="${codecs.join(', ')}"`;
}

function fallbackMimeType(name: string, kind: MediaKind): string {
  const ext = name.split('.').pop()?.toLowerCase();
  const byExt: Record<string, string> = {
    aac: 'audio/aac',
    flac: 'audio/flac',
    m4a: 'audio/mp4',
    m4v: 'video/mp4',
    mkv: 'video/x-matroska',
    mov: 'video/quicktime',
    mp3: 'audio/mpeg',
    mp4: 'video/mp4',
    ogg: 'audio/ogg',
    opus: 'audio/ogg',
    wav: 'audio/wav',
    webm: 'video/webm',
    wma: 'audio/x-ms-wma',
    wmv: 'video/x-ms-wmv',
  };

  return byExt[ext || ''] || `${kind}/*`;
}

function canPlay(kind: MediaKind, contentType: string): boolean {
  if (!contentType || contentType.endsWith('/*')) return false;
  const mediaEl = document.createElement(kind);
  return mediaEl.canPlayType(contentType) !== '';
}

function estimateBitrate(entry: FileEntry): number | null {
  const durationMs = entry.media_info?.duration_ms;
  if (!durationMs || durationMs <= 0 || !entry.size) return null;
  return Math.round((entry.size * 8 * 1000) / durationMs);
}

function videoBitrateLimit(): number {
  const nav = navigator as Navigator & { connection?: { downlink?: number } };
  const downlink = nav.connection?.downlink;
  if (typeof downlink === 'number' && Number.isFinite(downlink) && downlink > 0) {
    return Math.min(32_000_000, downlink * 1_000_000 * 0.5);
  }

  return 12_000_000;
}

function Diagnostics({
  session,
  decision,
  status,
  playerError,
  mediaInfo,
}: {
  session: string;
  decision: MediaDecision;
  status: PreviewStatus | null;
  playerError: string | null;
  mediaInfo: FileEntry['media_info'] | null;
}) {
  const [copied, setCopied] = useState(false);
  const details = {
    session,
    decision,
    status,
    player_error: playerError,
    media_info: mediaInfo,
  };
  const detailText = JSON.stringify(details, null, 2);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const handleCopyDetails = useCallback(() => {
    navigator.clipboard
      ?.writeText(detailText)
      .then(() => setCopied(true))
      .catch(() => undefined);
  }, [detailText]);

  return (
    <div
      data-preview-no-close
      style={{
        width: 'min(760px, 92vw)',
        maxHeight: 220,
        overflow: 'auto',
        border: '1px solid rgba(255,255,255,0.14)',
        borderRadius: 'var(--radius-md)',
        background: 'rgba(0,0,0,0.34)',
        color: '#fff',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 'var(--space-3)',
          padding: 'var(--space-2) var(--space-3)',
          borderBottom: '1px solid rgba(255,255,255,0.1)',
        }}
      >
        <div style={{ fontSize: 'var(--text-xs)', color: 'rgba(255,255,255,0.72)' }}>
          Diagnostics {status ? `- ${status.state}` : ''}
        </div>
        <button
          type="button"
          onClick={handleCopyDetails}
          style={smallButtonStyle}
          title={copied ? 'Copied' : 'Copy details'}
        >
          {copied && <Icon name="checkCircle" size={13} color="rgba(110, 230, 160, 0.95)" />}
          <span>{copied ? 'Copied' : 'Copy details'}</span>
        </button>
      </div>
      <pre
        style={{
          margin: 0,
          padding: 'var(--space-3)',
          color: 'rgba(255,255,255,0.78)',
          fontSize: 'var(--text-xs)',
          whiteSpace: 'pre-wrap',
          overflowWrap: 'anywhere',
        }}
      >
        {detailText}
      </pre>
    </div>
  );
}

function MediaBadge({
  label,
  tone,
  onClick,
  pressed,
}: {
  label: string;
  tone: 'ok' | 'preview' | 'error';
  onClick?: () => void;
  pressed?: boolean;
}) {
  const color =
    tone === 'ok'
      ? 'rgba(50, 190, 120, 0.24)'
      : tone === 'preview'
        ? 'rgba(90, 150, 255, 0.25)'
        : 'rgba(220, 70, 70, 0.25)';
  const style: React.CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    minHeight: 24,
    padding: '0 var(--space-2)',
    borderRadius: 'var(--radius-sm)',
    border: pressed ? '1px solid rgba(255,255,255,0.34)' : '1px solid rgba(255,255,255,0.16)',
    background: color,
    color: '#fff',
    fontSize: 'var(--text-xs)',
    fontWeight: 600,
    whiteSpace: 'nowrap',
    fontFamily: 'inherit',
    lineHeight: 1,
  };

  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        aria-pressed={pressed}
        title={pressed ? 'Hide diagnostics' : 'Show diagnostics'}
        style={{
          ...style,
          cursor: 'pointer',
          appearance: 'none',
        }}
      >
        {label}
      </button>
    );
  }

  return (
    <span
      style={style}
    >
      {label}
    </span>
  );
}

function createPreviewSession(): string {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return `preview-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function formatBitrate(bitsPerSecond: number): string {
  if (bitsPerSecond >= 1_000_000) return `${(bitsPerSecond / 1_000_000).toFixed(1)} Mbps`;
  return `${Math.round(bitsPerSecond / 1000)} kbps`;
}

const panelStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  justifyContent: 'center',
  gap: 'var(--space-3)',
  color: '#fff',
  textAlign: 'center',
};

const titleStyle: React.CSSProperties = {
  maxWidth: 'min(560px, 86vw)',
  color: '#fff',
  fontSize: 'var(--text-lg)',
  fontWeight: 600,
  overflowWrap: 'anywhere',
};

const mutedTextStyle: React.CSSProperties = {
  margin: 0,
  color: 'rgba(255,255,255,0.62)',
  fontSize: 'var(--text-sm)',
};

const timelineHintStyle: React.CSSProperties = {
  minHeight: 20,
  color: 'rgba(255,255,255,0.58)',
  fontSize: 'var(--text-xs)',
  textAlign: 'center',
};

const smallButtonStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 'var(--space-1)',
  border: '1px solid rgba(255,255,255,0.16)',
  borderRadius: 'var(--radius-sm)',
  background: 'rgba(255,255,255,0.08)',
  color: '#fff',
  cursor: 'pointer',
  fontSize: 'var(--text-xs)',
  padding: 'var(--space-1) var(--space-2)',
};
