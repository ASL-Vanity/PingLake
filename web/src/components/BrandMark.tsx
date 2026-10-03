import { useId } from "react";
import type { CSSProperties, SVGProps } from "react";

export interface BrandMarkProps {
  /** Preset size or an exact pixel size for the mark. */
  size?: "sm" | "md" | "lg" | number;
  /** Include the PingLake wordmark beside the signal icon. */
  showWordmark?: boolean;
  className?: string;
  title?: string;
  wordmarkClassName?: string;
}

const sizeMap = { sm: 28, md: 38, lg: 52 } as const;

function resolveSize(size: BrandMarkProps["size"]): number {
  if (typeof size === "number") return size;
  return sizeMap[size ?? "md"];
}

/**
 * PingLake's selected G mark: four monitored nodes joined through a shared
 * center. The primary color follows the active theme and the lower pair uses
 * the theme's live signal color.
 */
export function BrandMark({
  size = "md",
  showWordmark = false,
  className,
  title,
  wordmarkClassName,
}: BrandMarkProps) {
  const pixels = resolveSize(size);
  const titleId = `${useId().replace(/:/g, "")}-title`;
  const rootStyle: CSSProperties = { display: "inline-flex", alignItems: "center", gap: "0.65rem" };
  const iconProps: SVGProps<SVGSVGElement> = {
    width: pixels,
    height: pixels,
    viewBox: "0 0 64 64",
    fill: "none",
    role: title ? "img" : "presentation",
    "aria-hidden": title ? undefined : true,
    "aria-labelledby": title ? titleId : undefined,
    focusable: "false",
  };

  return (
    <span className={className} style={rootStyle}>
      <svg {...iconProps}>
        {title ? <title id={titleId}>{title}</title> : null}
        <path d="M17 17 47 47M47 17 17 47" stroke="currentColor" strokeWidth="4.5" strokeLinecap="round" />
        <circle cx="17" cy="17" r="6" fill="currentColor" />
        <circle cx="47" cy="17" r="6" fill="currentColor" />
        <circle cx="17" cy="47" r="6" fill="var(--green, #55d6a2)" />
        <circle cx="47" cy="47" r="6" fill="var(--green, #55d6a2)" />
      </svg>
      {showWordmark ? <span className={wordmarkClassName ?? "brand-wordmark"}>PingLake</span> : null}
    </span>
  );
}

export default BrandMark;
