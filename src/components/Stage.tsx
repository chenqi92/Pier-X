import type { ReactNode } from "react";

type Props = {
  children: ReactNode;
};

/**
 * Stage wraps the application chrome so we can render a window-in-viewport
 * frame (rounded corners + soft shadow + radial background glow) in dev,
 * matching the Remix reference's `.stage` + `.app` composition.
 *
 * In the native Tauri window this layer is visually subtle — the stage
 * background shows through only at the rounded corners when the user
 * shrinks the window below the app max-width. On fullscreen it is a no-op.
 */
export default function Stage({ children }: Props) {
  return <div className="stage">{children}</div>;
}
