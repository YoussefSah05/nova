//! Small presentational primitives shared by the settings sections.

import { cn } from "@/lib/utils";
import { Switch } from "@/components/ui/switch";
import { SimpleSelect } from "@/components/ui/select";

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

export function SettingsCard({ title, children }: { title?: string; children: React.ReactNode }) {
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
  return <Switch checked={checked} onCheckedChange={onChange} aria-label={label} />;
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
  return <SimpleSelect value={value} options={options} onChange={onChange} ariaLabel={ariaLabel} />;
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
      className="w-full rounded-lg border border-input bg-white/[0.04] px-3 py-1.5 text-sm text-white placeholder:text-white/30 focus:outline-none focus:ring-2 focus:ring-ring"
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
        variant === "primary" && "bg-primary text-primary-foreground hover:brightness-110",
        variant === "danger" && "bg-rose-500/15 text-rose-300 hover:bg-rose-500/25",
        variant === "default" && "bg-white/10 text-white/85 hover:bg-white/20"
      )}
    >
      {children}
    </button>
  );
}
