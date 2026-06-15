// ── ComboInput — free-text input with suggestions (shared atom) ────
// Replacement for `<input list>` + `<datalist>`: WebView2 draws the
// datalist popup natively (white, unstylable) AND adds its own arrow
// glyph inside the input. This keeps the plain input semantics —
// typing any value works, suggestions are optional — and renders the
// suggestion list as the same `.ui-pop` popover the Select atom uses.

import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { createPortal } from "react-dom";

type Props = {
  value: string;
  onChange: (value: string) => void;
  suggestions: string[];
  className?: string;
  style?: CSSProperties;
  placeholder?: string;
  disabled?: boolean;
  title?: string;
  mono?: boolean;
  /** Forwarded so call sites can keep their commit-on-Enter flows.
   *  Fires only when the popover did NOT consume the key. */
  onKeyDown?: (e: ReactKeyboardEvent<HTMLInputElement>) => void;
  onBlur?: () => void;
  autoFocus?: boolean;
};

const POP_MAX_H = 280;
const POP_MARGIN = 8;
const MAX_VISIBLE = 200;

export default function ComboInput({
  value,
  onChange,
  suggestions,
  className,
  style,
  placeholder,
  disabled,
  title,
  mono,
  onKeyDown,
  onBlur,
  autoFocus,
}: Props) {
  const [open, setOpen] = useState(false);
  const [hl, setHl] = useState(-1);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const [popStyle, setPopStyle] = useState<CSSProperties>({});

  // Substring filter (case-insensitive); exact-prefix matches sort
  // first so short inputs surface the natural candidates on top.
  const filtered = useMemo(() => {
    const q = value.trim().toLowerCase();
    let list = suggestions;
    if (q) {
      const starts: string[] = [];
      const contains: string[] = [];
      for (const s of suggestions) {
        const l = s.toLowerCase();
        if (l.startsWith(q)) starts.push(s);
        else if (l.includes(q)) contains.push(s);
      }
      list = [...starts, ...contains];
    }
    return list.slice(0, MAX_VISIBLE);
  }, [suggestions, value]);

  useLayoutEffect(() => {
    if (!open) return;
    const el = inputRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - POP_MARGIN;
    const above = r.top - POP_MARGIN;
    const flip = below < 140 && above > below;
    const maxH = Math.min(POP_MAX_H, flip ? above : below);
    const width = Math.max(r.width, 140);
    const left = Math.min(r.left, window.innerWidth - width - POP_MARGIN);
    setPopStyle({
      left: Math.max(POP_MARGIN, left),
      minWidth: r.width,
      maxHeight: Math.max(80, maxH),
      ...(flip
        ? { bottom: window.innerHeight - r.top + 2 }
        : { top: r.bottom + 2 }),
    });
  }, [open, filtered.length]);

  useEffect(() => {
    if (!open || hl < 0) return;
    const el = popRef.current?.querySelector<HTMLElement>(`[data-idx="${hl}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [open, hl]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (popRef.current?.contains(t) || inputRef.current?.contains(t)) return;
      setOpen(false);
    };
    const onScroll = (e: Event) => {
      if (popRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    const onResize = () => setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  const pick = (s: string) => {
    onChange(s);
    setOpen(false);
    setHl(-1);
    inputRef.current?.focus();
  };

  const handleKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (open && filtered.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setHl((h) => (h + 1) % filtered.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setHl((h) => (h <= 0 ? filtered.length - 1 : h - 1));
        return;
      }
      if (e.key === "Enter" && hl >= 0) {
        e.preventDefault();
        pick(filtered[hl]);
        return;
      }
      if (e.key === "Escape") {
        // Only the popover closes — not the dialog hosting us.
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        setHl(-1);
        return;
      }
      if (e.key === "Tab") {
        setOpen(false);
        setHl(-1);
      }
    } else if (e.key === "ArrowDown" && !open && filtered.length > 0) {
      e.preventDefault();
      setOpen(true);
      setHl(0);
      return;
    }
    onKeyDown?.(e);
  };

  return (
    <>
      <input
        ref={inputRef}
        className={"ui-combo" + (mono ? " is-mono" : "") + (className ? ` ${className}` : "")}
        style={style}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        title={title}
        autoFocus={autoFocus}
        autoComplete="off"
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        onChange={(e) => {
          onChange(e.currentTarget.value);
          if (suggestions.length > 0) {
            setOpen(true);
            setHl(-1);
          }
        }}
        onFocus={() => {
          if (suggestions.length > 0) setOpen(true);
        }}
        onBlur={() => {
          setOpen(false);
          setHl(-1);
          onBlur?.();
        }}
        onKeyDown={handleKeyDown}
      />
      {open && filtered.length > 0 &&
        createPortal(
          <div ref={popRef} className="ui-pop" style={popStyle} role="listbox">
            {filtered.map((s, i) => (
              <button
                key={`${s}-${i}`}
                type="button"
                data-idx={i}
                className={
                  "ui-pop__item is-plain" +
                  (i === hl ? " is-hl" : "") +
                  (s === value ? " is-active" : "")
                }
                tabIndex={-1}
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => setHl(i)}
                onClick={() => pick(s)}
              >
                <span className="ui-pop__label">{s}</span>
              </button>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
