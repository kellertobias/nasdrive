import { useId } from "react";

interface AppLogoProps {
  size?: number;
  showWordmark?: boolean;
  wordmarkSize?: number;
  compact?: boolean;
}

export function AppLogo({
  size = 28,
  showWordmark = true,
  wordmarkSize = 16,
  compact = false,
}: AppLogoProps) {
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: compact ? 8 : 10,
        color: "var(--color-fg)",
        lineHeight: 1,
      }}
    >
      <LogoMark size={size} />
      {showWordmark && (
        <span
          style={{
            fontSize: wordmarkSize,
            fontWeight: 750,
            letterSpacing: "var(--tracking-normal)",
          }}
        >
          NASDrive
        </span>
      )}
    </span>
  );
}

function LogoMark({ size }: { size: number }) {
  const id = useId().replace(/:/g, "");
  const bgId = `${id}-bg`;
  const pageId = `${id}-page`;

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      role="img"
      aria-label="NASDrive"
      style={{ display: "block", flexShrink: 0 }}
    >
      <defs>
        <linearGradient
          id={bgId}
          x1="12"
          y1="9"
          x2="52"
          y2="55"
          gradientUnits="userSpaceOnUse"
        >
          <stop stopColor="#5f7cff" />
          <stop offset="0.52" stopColor="#31b5c9" />
          <stop offset="1" stopColor="#36c17b" />
        </linearGradient>
        <linearGradient
          id={pageId}
          x1="21"
          y1="16"
          x2="44"
          y2="47"
          gradientUnits="userSpaceOnUse"
        >
          <stop stopColor="#ffffff" />
          <stop offset="1" stopColor="#dff7f2" />
        </linearGradient>
      </defs>
      <rect x="5" y="7" width="54" height="50" rx="13" fill={`url(#${bgId})`} />
      <path
        d="M16 25.5C16 22.46 18.46 20 21.5 20h7.72c1.26 0 2.47.5 3.36 1.4l2.02 2.02c.89.89 2.1 1.39 3.36 1.39H45.5c3.04 0 5.5 2.46 5.5 5.5V43c0 3.31-2.69 6-6 6H22c-3.31 0-6-2.69-6-6V25.5Z"
        fill="#0f2630"
        opacity="0.22"
      />
      <path
        d="M14 23.5C14 20.46 16.46 18 19.5 18h8.03c1.23 0 2.41.49 3.28 1.36l2.33 2.33c.87.87 2.05 1.36 3.28 1.36H44.5c3.04 0 5.5 2.46 5.5 5.5V41c0 3.31-2.69 6-6 6H20c-3.31 0-6-2.69-6-6V23.5Z"
        fill={`url(#${pageId})`}
      />
      <path
        d="M24 32.5h16M24 38.5h11"
        stroke="#26767b"
        strokeWidth="3"
        strokeLinecap="round"
      />
      <path
        d="M45 18.5v-4.25M45 14.25h-4.25M45 14.25l-7 7"
        stroke="#e9fff8"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.95"
      />
    </svg>
  );
}
