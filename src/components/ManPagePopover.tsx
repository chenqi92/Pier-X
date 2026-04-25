// ── Man-page popover (M6) ──────────────────────────────────────────
//
// Smart-mode help popover triggered by Ctrl+Shift+M. Shows the
// SYNOPSIS + DESCRIPTION + OPTIONS sections of the command at the
// cursor without leaving the terminal — saves the user a context
// switch to a separate `man` invocation.
//
// All chrome is reused from the existing `.popover` atom +
// `popover-section` / `popover-item` rows. Option flags use the
// shared `Pill` atom so they pick up the same styling as
// `--info` chips elsewhere. No new dialog colours.
//
// Three visible states:
//   * loading  — fired immediately on open while the backend
//                spawns `man -P cat`, so the user sees the popover
//                snap into place even when the actual lookup takes
//                a few hundred ms.
//   * empty    — `terminal_man_synopsis` resolved with `null`; the
//                command has no man page and no usable `--help`.
//   * loaded   — render the three sections.

import Popover from "./Popover";
import Pill from "./Pill";
import type { ManSynopsis } from "../lib/terminalSmart";

type Props = {
  open: boolean;
  anchor: HTMLElement | null;
  /** Command name the user invoked help for. Always rendered in
   *  the popover header so the user can verify which command we
   *  looked up — useful when the cursor was ambiguous about which
   *  word to grab. */
  command: string;
  /** `null` while the request is in flight or returned no data;
   *  populated with parsed sections on success. The component uses
   *  `loading` to disambiguate the two null cases. */
  data: ManSynopsis | null;
  loading: boolean;
  errorMessage?: string | null;
  onClose: () => void;
};

export default function ManPagePopover({
  open,
  anchor,
  command,
  data,
  loading,
  errorMessage,
  onClose,
}: Props) {
  if (!open) return null;

  return (
    <Popover
      open={open}
      anchor={anchor}
      onClose={onClose}
      placement="bottom-start"
      width={520}
      closeOnScroll={false}
      className="man-popover"
    >
      <div className="popover-header">
        <span className="popover-header__title">man · {command}</span>
        {data?.source ? (
          <span className="popover-header__sub">
            {data.source === "man" ? "man -P cat" : "--help"}
          </span>
        ) : null}
      </div>

      {loading ? (
        <div className="popover-section man-popover__placeholder">
          Loading…
        </div>
      ) : errorMessage ? (
        <div className="popover-section man-popover__placeholder">
          {errorMessage}
        </div>
      ) : !data ? (
        <div className="popover-section man-popover__placeholder">
          No documentation found for {command}.
        </div>
      ) : (
        <>
          {data.synopsis ? (
            <ManSection title="SYNOPSIS">
              <pre className="man-popover__pre">{data.synopsis}</pre>
            </ManSection>
          ) : null}

          {data.description ? (
            <ManSection title="DESCRIPTION">
              <p className="man-popover__paragraph">{data.description}</p>
            </ManSection>
          ) : null}

          {data.options.length > 0 ? (
            <ManSection title={`OPTIONS · ${data.options.length}`}>
              <div className="man-popover__options">
                {data.options.map((opt, i) => (
                  <div
                    key={`${opt.flag}-${i}`}
                    className="man-popover__option-row"
                  >
                    <Pill tinted className="man-popover__option-flag">
                      {opt.flag}
                    </Pill>
                    <span className="man-popover__option-summary">
                      {opt.summary || "(no description)"}
                    </span>
                  </div>
                ))}
              </div>
            </ManSection>
          ) : null}
        </>
      )}
    </Popover>
  );
}

function ManSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="man-popover__section">
      <div className="popover-section">{title}</div>
      <div className="man-popover__section-body">{children}</div>
    </div>
  );
}
