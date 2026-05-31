//! Small presentational primitives shared by the settings sections.

import { cn } from "@/lib/utils";

/// A labelled row: title (+ optional description) on the left, control on the right.
export function SettingRow({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6 py-3">
      <div className="min-w-0">
        <p className="text-sm font-medium text-white">{title}</p>
        {description && <p className="mt-0.5 text-xs text-white/50">{description}</p>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

export function SettingsCard({
  title,
  children,
}: {
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-xl border border-white/10 bg-white/[0.02] px-4">
      {title && (
        <p className="border-b border-white/5 py-3 text-[11px] font-semibold uppercase tracking-wider text-white/45">
          {title}
        </p>
      )}
      <div className="divide-y divide-white/5">{children}</div>
    </section>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-6 w-11 shrink-0 rounded-full transition-colors",
        checked ? "bg-[var(--color-accent)]" : "bg-white/15"
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 h-5 w-5 rounded-full bg-white transition-transform",
          checked ? "translate-x-[1.375rem]" : "translate-x-0.5"
        )}
      />
    </button>
  );
}

export function SelectInput<T extends string>({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  ariaLabel?: string;
}) {
  return (
    <select
      aria-label={ariaLabel}
      value={value}
      onChange={(e) => onChange(e.target.value as T)}
      className="rounded-lg border border-white/10 bg-neutral-800 px-3 py-1.5 text-sm text-white focus:outline-none focus:ring-2 focus:ring-indigo-400"
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function TextInput({
  value,
  onChange,
  placeholder,
  ariaLabel,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  ariaLabel?: string;
}) {
  return (
    <input
      type="text"
      value={value}
      placeholder={placeholder}
      aria-label={ariaLabel}
      onChange={(e) => onChange(e.target.value)}
      className="w-full rounded-lg border border-white/10 bg-neutral-800 px-3 py-1.5 text-sm text-white placeholder:text-white/30 focus:outline-none focus:ring-2 focus:ring-indigo-400"
    />
  );
}

export function SettingsButton({
  children,
  onClick,
  variant = "default",
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  variant?: "default" | "primary" | "danger";
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "rounded-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        variant === "primary" && "bg-[var(--color-accent)] text-white hover:brightness-110",
        variant === "danger" && "bg-rose-500/15 text-rose-300 hover:bg-rose-500/25",
        variant === "default" && "bg-white/10 text-white/85 hover:bg-white/20"
      )}
    >
      {children}
    </button>
  );
}
