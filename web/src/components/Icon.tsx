import { ICONS } from "../lib/icons";

interface IconProps {
  name: keyof typeof ICONS;
  size?: number;
  color?: string;
  className?: string;
  style?: React.CSSProperties;
}

/**
 * Renders an inline SVG icon from the ICONS dictionary.
 * Inherits `currentColor` by default so it matches surrounding text.
 */
export function Icon({ name, size = 18, color, className, style }: IconProps) {
  return (
    <span
      className={className}
      aria-hidden="true"
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: size,
        height: size,
        flexShrink: 0,
        color: color || "currentColor",
        ...style,
      }}
      dangerouslySetInnerHTML={{ __html: ICONS[name] }}
    />
  );
}

interface FileIconProps {
  svg: string;
  color: string;
  size?: number;
  className?: string;
}

/**
 * Renders a file-type icon using the pre-resolved SVG string from getFileIcon().
 */
export function FileIcon({ svg, color, size = 18, className }: FileIconProps) {
  return (
    <span
      className={className}
      aria-hidden="true"
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: size,
        height: size,
        flexShrink: 0,
        color,
      }}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
