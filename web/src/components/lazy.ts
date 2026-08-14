import { lazy } from "react";

/**
 * Code-split entry points for the heavy, rarely-needed parts of the UI.
 *
 * The app used to ship as a single ~4.2MB JavaScript chunk, because video.js,
 * its HLS plugin, CodeMirror and the markdown renderer were all reachable from
 * the entry module. Every visit downloaded and parsed a video player before it
 * could draw a list of filenames. None of these are needed to browse files —
 * they load when a preview actually opens.
 *
 * Each component here must be rendered inside a `<Suspense>` boundary.
 */

export const MediaPreview = lazy(() =>
  import("./MediaPreview").then((m) => ({ default: m.MediaPreview })),
);

export const CodeViewer = lazy(() =>
  import("./CodeViewer").then((m) => ({ default: m.CodeViewer })),
);

export const DirectoryReadme = lazy(() =>
  import("./DirectoryReadme").then((m) => ({ default: m.DirectoryReadme })),
);
