import type { SVGProps } from "react";

type Props = SVGProps<SVGSVGElement> & { size?: number | string };

/**
 * NanoLink brand glyph — the stylized "N" single-stroke mark from
 * NanoLink's logo (`apps/desktop/assets/logo.svg`), reduced to one
 * monochrome stroke plus its two node dots so it inherits the tool-rail
 * tint via `stroke`/`fill="currentColor"`. The logo's neon gradient,
 * dark plate, and grid are brand chrome that can't tint, so they're
 * dropped — same approach as `DockerIcon` (mono `currentColor`).
 *
 * API-compatible with `LucideIcon` (see `lib/rightToolMeta.ts`): accepts
 * `size` plus any standard SVG attribute.
 */
export default function NanoLinkIcon({ size = 24, ...props }: Props) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      /* Cropped tight to the glyph (the stroke spans x[278,746] y[222,802],
         centred on 512,512) so the "N" fills the icon slot like the lucide
         siblings instead of floating small in a 1024 canvas. */
      viewBox="192 192 640 640"
      fill="none"
      aria-hidden="true"
      {...props}
    >
      <path
        d="M320 760V340C320 260 360 260 420 340L604 684C664 764 704 764 704 684V264"
        stroke="currentColor"
        strokeWidth={64}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="320" cy="760" r="40" fill="currentColor" />
      <circle cx="704" cy="264" r="40" fill="currentColor" />
    </svg>
  );
}
