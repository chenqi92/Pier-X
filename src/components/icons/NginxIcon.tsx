import type { SVGProps } from "react";

type Props = SVGProps<SVGSVGElement> & { size?: number | string };

/** NGINX wordmark mark — the "N" inside a hexagon, simplified to a
 * single-color glyph that takes `currentColor` so it tints with
 * `--svc-nginx` like the other service icons. */
export default function NginxIcon({ size = 24, ...props }: Props) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      <path d="M12 2 3 7v10l9 5 9-5V7z" />
      <path d="M9 16V8l6 8V8" />
    </svg>
  );
}
