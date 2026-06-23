import type { FileEntry } from "../api/client";

// ---------------------------------------------------------------------------
// Flat, monochrome SVG icon library — every icon is a single-color path
// designed at 24×24. Icons render as inline SVG strings so they can be
// embedded via dangerouslySetInnerHTML or used in the <Icon> component.
// ---------------------------------------------------------------------------

// Helper: wrap a viewBox-24 path in an SVG shell. `fill=currentColor` lets
// the parent's CSS `color` property control the icon color.
const svg = (d: string, strokeBased = false): string =>
  strokeBased
    ? `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`
    : `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;

// ---- Core icons ----

export const ICONS = {
  // Navigation & UI
  folder: svg(
    '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  ),
  folderOpen: svg(
    '<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>',
  ),
  folders: svg(
    '<path d="M20 17a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3.9a2 2 0 0 1-1.69-.9l-.81-1.2a2 2 0 0 0-1.67-.9H8a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2Z"/><path d="M2 8v11a2 2 0 0 0 2 2h14"/>',
  ),
  home: svg(
    '<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-5.999a2 2 0 0 1 2.582 0l7 5.999A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  ),
  chevronRight: svg('<path d="m9 18 6-6-6-6"/>'),
  chevronDown: svg('<path d="m6 9 6 6 6-6"/>'),
  arrowLeft: svg('<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>'),
  menu: svg(
    '<line x1="4" x2="20" y1="12" y2="12"/><line x1="4" x2="20" y1="6" y2="6"/><line x1="4" x2="20" y1="18" y2="18"/>',
  ),
  search: svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'),
  grid: svg(
    '<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>',
  ),
  list: svg(
    '<line x1="8" x2="21" y1="6" y2="6"/><line x1="8" x2="21" y1="12" y2="12"/><line x1="8" x2="21" y1="18" y2="18"/><line x1="3" x2="3.01" y1="6" y2="6"/><line x1="3" x2="3.01" y1="12" y2="12"/><line x1="3" x2="3.01" y1="18" y2="18"/>',
  ),
  columns: svg(
    '<rect x="3" y="4" width="5" height="16" rx="1"/><rect x="10" y="4" width="5" height="16" rx="1"/><rect x="17" y="4" width="4" height="16" rx="1"/>',
  ),
  logout: svg(
    '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" x2="9" y1="12" y2="12"/>',
  ),
  user: svg(
    '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>',
  ),
  settings: svg(
    '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.5a2 2 0 0 1-1 1.72l-.15.1a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.1a2 2 0 0 1-1-1.72v-.5a2 2 0 0 1 1-1.72l.15-.1a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  ),
  x: svg('<path d="M18 6 6 18"/><path d="m6 6 12 12"/>'),
  checkCircle: svg(
    '<path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><path d="m9 11 3 3L22 4"/>',
  ),
  share2: svg(
    '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" x2="15.42" y1="13.51" y2="17.49"/><line x1="15.41" x2="8.59" y1="6.51" y2="10.49"/>',
  ),
  download: svg(
    '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/>',
  ),
  hardDrive: svg(
    '<line x1="22" x2="2" y1="12" y2="12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><line x1="6" x2="6.01" y1="16" y2="16"/><line x1="10" x2="10.01" y1="16" y2="16"/>',
  ),
  upload: svg(
    '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" x2="12" y1="3" y2="15"/>',
  ),

  // File types
  file: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
  ),
  fileText: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><line x1="8" x2="16" y1="13" y2="13"/><line x1="8" x2="16" y1="17" y2="17"/><line x1="8" x2="10" y1="9" y2="9"/>',
  ),
  fileCode: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="m10 13-2 2 2 2"/><path d="m14 17 2-2-2-2"/>',
  ),
  fileJson: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 12a1 1 0 0 0-1 1v1a1 1 0 0 1-1 1 1 1 0 0 1 1 1v1a1 1 0 0 0 1 1"/><path d="M14 18a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1 1 1 0 0 1-1-1v-1a1 1 0 0 0-1-1"/>',
  ),
  fileSpreadsheet: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M8 13h2"/><path d="M14 13h2"/><path d="M8 17h2"/><path d="M14 17h2"/>',
  ),
  filePresentation: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><rect x="8" y="12" width="8" height="6" rx="1"/>',
  ),
  fileLock: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><rect x="8" y="14" width="8" height="5" rx="1"/><path d="M10 14v-2a2 2 0 1 1 4 0v2"/>',
  ),

  // Media
  image: svg(
    '<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>',
  ),
  video: svg(
    '<path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.934a.5.5 0 0 0-.777-.416L16 11"/><rect x="2" y="6" width="14" height="12" rx="2"/>',
  ),
  music: svg(
    '<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>',
  ),

  // Documents
  fileType: svg(
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 13v-1h6v1"/><path d="M12 12v6"/><path d="M11 18h2"/>',
  ),

  // Archives
  archive: svg(
    '<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>',
  ),

  // Terminal / executable
  terminal: svg(
    '<polyline points="4 17 10 11 4 5"/><line x1="12" x2="20" y1="19" y2="19"/>',
  ),

  // Markdown
  bookOpen: svg(
    '<path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/>',
  ),

  // Misc
  scrollText: svg(
    '<path d="M15 12h-5"/><path d="M15 8h-5"/><path d="M19 17V5a2 2 0 0 0-2-2H4"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2"/>',
  ),

  // Alert / empty
  folderSearch: svg(
    '<circle cx="17" cy="17" r="3"/><path d="m21 21-1.5-1.5"/><path d="M11 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4"/>',
  ),
  alertTriangle: svg(
    '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
  ),

  // Links
  link: svg(
    '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
  ),
  externalLink: svg(
    '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  ),
  globe: svg(
    '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>',
  ),
  star: svg(
    '<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>',
  ),
  heart: svg(
    '<path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/>',
  ),
  zap: svg(
    '<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>',
  ),
  mail: svg(
    '<rect width="20" height="16" x="2" y="4" rx="2"/><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/>',
  ),
  phone: svg(
    '<path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07A19.5 19.5 0 0 1 4.72 13 19.79 19.79 0 0 1 1.65 4.35 2 2 0 0 1 3.62 2h3a2 2 0 0 1 2 1.72c.127.96.361 1.903.7 2.81a2 2 0 0 1-.45 2.11L7.91 9.91a16 16 0 0 0 6.1 6.1l.91-.91a2 2 0 0 1 2.11-.45c.907.339 1.85.573 2.81.7A2 2 0 0 1 22 16.92z"/>',
  ),
  calendar: svg(
    '<path d="M8 2v4"/><path d="M16 2v4"/><rect width="18" height="18" x="3" y="4" rx="2"/><path d="M3 10h18"/>',
  ),
  clock: svg(
    '<circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>',
  ),
  map: svg(
    '<path d="M14.106 5.553a2 2 0 0 0 1.788 0l3.659-1.83A1 1 0 0 1 21 4.619v12.764a1 1 0 0 1-.553.894l-4.553 2.277a2 2 0 0 1-1.788 0l-4.212-2.106a2 2 0 0 0-1.788 0l-3.659 1.83A1 1 0 0 1 3 19.381V6.618a1 1 0 0 1 .553-.894l4.553-2.277a2 2 0 0 1 1.788 0z"/><path d="M15 5.764v15"/><path d="M9 3.236v15"/>',
  ),
  bookmark: svg(
    '<path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/>',
  ),
  bell: svg(
    '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
  ),
  database: svg(
    '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/>',
  ),
  server: svg(
    '<rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/>',
  ),
  cloud: svg('<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>'),
  shield: svg(
    '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>',
  ),
  key: svg(
    '<circle cx="7.5" cy="15.5" r="5.5"/><path d="m21 2-9.6 9.6"/><path d="m15.5 7.5 3 3L22 7l-3-3"/>',
  ),
  lock: svg(
    '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  ),
  tool: svg(
    '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>',
  ),
  cpu: svg(
    '<rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M15 2v2"/><path d="M15 20v2"/><path d="M2 15h2"/><path d="M2 9h2"/><path d="M20 15h2"/><path d="M20 9h2"/><path d="M9 2v2"/><path d="M9 20v2"/>',
  ),
  package: svg(
    '<path d="m7.5 4.27 9 5.15"/><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/>',
  ),
  copy: svg(
    '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
  ),
  check: svg('<path d="M20 6 9 17l-5-5"/>'),
} as const;

// ---------------------------------------------------------------------------
// Extension → icon + color mapping
// ---------------------------------------------------------------------------

interface IconDef {
  svgKey: keyof typeof ICONS;
  color: string;
}

const EXTENSION_ICONS: Record<string, IconDef> = {
  // Images
  jpg: { svgKey: "image", color: "#e06b8a" },
  jpeg: { svgKey: "image", color: "#e06b8a" },
  png: { svgKey: "image", color: "#e06b8a" },
  gif: { svgKey: "image", color: "#e06b8a" },
  webp: { svgKey: "image", color: "#e06b8a" },
  svg: { svgKey: "image", color: "#e06b8a" },
  bmp: { svgKey: "image", color: "#e06b8a" },
  ico: { svgKey: "image", color: "#e06b8a" },
  avif: { svgKey: "image", color: "#e06b8a" },
  heic: { svgKey: "image", color: "#e06b8a" },

  // Video
  mp4: { svgKey: "video", color: "#8b7cf6" },
  mkv: { svgKey: "video", color: "#8b7cf6" },
  avi: { svgKey: "video", color: "#8b7cf6" },
  mov: { svgKey: "video", color: "#8b7cf6" },
  webm: { svgKey: "video", color: "#8b7cf6" },
  m4v: { svgKey: "video", color: "#8b7cf6" },
  wmv: { svgKey: "video", color: "#8b7cf6" },

  // Audio
  mp3: { svgKey: "music", color: "#d660a0" },
  flac: { svgKey: "music", color: "#d660a0" },
  wav: { svgKey: "music", color: "#d660a0" },
  aac: { svgKey: "music", color: "#d660a0" },
  ogg: { svgKey: "music", color: "#d660a0" },
  m4a: { svgKey: "music", color: "#d660a0" },
  wma: { svgKey: "music", color: "#d660a0" },

  // Documents
  pdf: { svgKey: "fileType", color: "#e04040" },
  doc: { svgKey: "fileText", color: "#4082e0" },
  docx: { svgKey: "fileText", color: "#4082e0" },
  odt: { svgKey: "fileText", color: "#4082e0" },
  rtf: { svgKey: "fileText", color: "#4082e0" },

  // Spreadsheets
  xls: { svgKey: "fileSpreadsheet", color: "#2da050" },
  xlsx: { svgKey: "fileSpreadsheet", color: "#2da050" },
  csv: { svgKey: "fileSpreadsheet", color: "#2da050" },
  ods: { svgKey: "fileSpreadsheet", color: "#2da050" },

  // Presentations
  ppt: { svgKey: "filePresentation", color: "#e07020" },
  pptx: { svgKey: "filePresentation", color: "#e07020" },
  odp: { svgKey: "filePresentation", color: "#e07020" },

  // Archives
  zip: { svgKey: "archive", color: "#8a8a8a" },
  tar: { svgKey: "archive", color: "#8a8a8a" },
  gz: { svgKey: "archive", color: "#8a8a8a" },
  "7z": { svgKey: "archive", color: "#8a8a8a" },
  rar: { svgKey: "archive", color: "#8a8a8a" },
  bz2: { svgKey: "archive", color: "#8a8a8a" },
  xz: { svgKey: "archive", color: "#8a8a8a" },

  // Code
  js: { svgKey: "fileCode", color: "#d4a017" },
  ts: { svgKey: "fileCode", color: "#3178c6" },
  jsx: { svgKey: "fileCode", color: "#5bbad5" },
  tsx: { svgKey: "fileCode", color: "#3178c6" },
  py: { svgKey: "fileCode", color: "#3572A5" },
  rs: { svgKey: "fileCode", color: "#c87a5a" },
  go: { svgKey: "fileCode", color: "#00ADD8" },
  java: { svgKey: "fileCode", color: "#b07219" },
  c: { svgKey: "fileCode", color: "#606060" },
  cpp: { svgKey: "fileCode", color: "#c05070" },
  h: { svgKey: "fileCode", color: "#606060" },
  rb: { svgKey: "fileCode", color: "#701516" },
  php: { svgKey: "fileCode", color: "#4F5D95" },
  swift: { svgKey: "fileCode", color: "#e04530" },
  kt: { svgKey: "fileCode", color: "#8070d0" },
  html: { svgKey: "fileCode", color: "#e04d1a" },
  css: { svgKey: "fileCode", color: "#2e6fcc" },
  scss: { svgKey: "fileCode", color: "#c66394" },

  // Config / data
  json: { svgKey: "fileJson", color: "#8a8a8a" },
  yaml: { svgKey: "fileText", color: "#8a8a8a" },
  yml: { svgKey: "fileText", color: "#8a8a8a" },
  toml: { svgKey: "fileText", color: "#8a8a8a" },
  xml: { svgKey: "fileCode", color: "#8a8a8a" },
  ini: { svgKey: "fileText", color: "#8a8a8a" },
  env: { svgKey: "fileLock", color: "#8a8a8a" },

  // Text
  txt: { svgKey: "fileText", color: "#8a8a8a" },
  md: { svgKey: "bookOpen", color: "#8a8a8a" },
  log: { svgKey: "scrollText", color: "#8a8a8a" },

  // Executables
  exe: { svgKey: "terminal", color: "#606060" },
  sh: { svgKey: "terminal", color: "#6aaa40" },
  bat: { svgKey: "terminal", color: "#8aaa20" },
};

const FOLDER_DEF: IconDef = { svgKey: "folder", color: "var(--color-accent)" };
const DEFAULT_DEF: IconDef = {
  svgKey: "file",
  color: "var(--color-fg-subtle)",
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface FileIconResult {
  svg: string; // inline SVG markup
  svgKey: string; // icon name (for caching / keying)
  color: string; // suggested color value
}

export function getFileIcon(entry: FileEntry): FileIconResult {
  if (entry.is_dir) {
    return {
      svg: ICONS[FOLDER_DEF.svgKey],
      svgKey: FOLDER_DEF.svgKey,
      color: FOLDER_DEF.color,
    };
  }
  const ext = entry.name.split(".").pop()?.toLowerCase() || "";
  const def = EXTENSION_ICONS[ext] || DEFAULT_DEF;
  return { svg: ICONS[def.svgKey], svgKey: def.svgKey, color: def.color };
}

export function getExtension(filename: string): string {
  const parts = filename.split(".");
  return parts.length > 1 ? parts[parts.length - 1].toLowerCase() : "";
}

export function formatFileSize(bytes: number): string {
  if (bytes === 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const value = bytes / Math.pow(1024, i);
  return `${value.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export function formatDate(timestamp: number): string {
  if (timestamp === 0) return "—";
  const date = new Date(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

  if (diffDays === 0) {
    return date.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  if (diffDays === 1) return "Yesterday";
  if (diffDays < 7) {
    return date.toLocaleDateString(undefined, { weekday: "short" });
  }
  if (date.getFullYear() === now.getFullYear()) {
    return date.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
  }
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function isSameLocalDate(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function twoDigits(value: number): string {
  return value.toString().padStart(2, "0");
}

export function formatModifiedDate(timestamp: number): string {
  if (timestamp === 0) return "—";

  const date = new Date(timestamp);
  const now = new Date();

  if (isSameLocalDate(date, now)) {
    return date.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  const day = twoDigits(date.getDate());
  const month = twoDigits(date.getMonth() + 1);
  const year = twoDigits(date.getFullYear() % 100);
  return `${day}.${month}.${year}`;
}

// ---------------------------------------------------------------------------
// Thumbnail / preview helpers
// ---------------------------------------------------------------------------

const THUMB_IMAGE_EXTS = new Set([
  "jpg",
  "jpeg",
  "png",
  "gif",
  "webp",
  "bmp",
  "tiff",
  "tif",
  "svg",
]);
const THUMB_VIDEO_EXTS = new Set([
  "mp4",
  "mkv",
  "avi",
  "mov",
  "webm",
  "m4v",
  "wmv",
  "flv",
]);
const PREVIEW_AUDIO_EXTS = new Set([
  "mp3",
  "ogg",
  "flac",
  "aac",
  "wav",
  "m4a",
  "wma",
  "m4b",
]);
const PREVIEW_PDF_EXTS = new Set(["pdf"]);
const PREVIEW_TEXT_EXTS = new Set([
  "txt",
  "md",
  "json",
  "yaml",
  "yml",
  "toml",
  "xml",
  "csv",
  "log",
  "py",
  "js",
  "ts",
  "tsx",
  "jsx",
  "rs",
  "go",
  "java",
  "c",
  "cpp",
  "h",
  "sh",
  "bash",
  "zsh",
  "fish",
  "css",
  "scss",
  "html",
  "htm",
  "sql",
  "rb",
  "php",
  "swift",
  "kt",
  "lua",
  "r",
  "pl",
  "conf",
  "ini",
  "env",
  "dockerfile",
  "makefile",
  "gitignore",
  "vtt",
]);

function getExt(entry: FileEntry): string {
  if (entry.is_dir) return "";
  const dot = entry.name.lastIndexOf(".");
  return dot >= 0 ? entry.name.slice(dot + 1).toLowerCase() : "";
}

/** True if the backend can generate a thumbnail for this file type. */
export function hasThumbnail(entry: FileEntry): boolean {
  return entry.has_thumbnail;
}

export type PreviewType = "image" | "video" | "audio" | "pdf" | "text" | null;

/** Determine the preview type for a file, or null if not previewable. */
export function getPreviewType(entry: FileEntry): PreviewType {
  if (entry.is_dir) return null;
  const ext = getExt(entry);
  if (THUMB_IMAGE_EXTS.has(ext)) return "image";
  if (THUMB_VIDEO_EXTS.has(ext)) return "video";
  if (PREVIEW_AUDIO_EXTS.has(ext)) return "audio";
  if (PREVIEW_PDF_EXTS.has(ext)) return "pdf";
  if (PREVIEW_TEXT_EXTS.has(ext)) return "text";
  // Check by name (no extension)
  const name = entry.name.toLowerCase();
  if (
    [
      "dockerfile",
      "makefile",
      ".gitignore",
      ".env",
      "readme",
      "license",
    ].includes(name)
  )
    return "text";
  return null;
}
