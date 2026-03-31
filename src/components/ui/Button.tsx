import React from "react";

interface ButtonProps {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md" | "lg";
  disabled?: boolean;
  loading?: boolean;
  icon?: React.ReactNode;
  onClick?: () => void;
  children: React.ReactNode;
  className?: string;
}

const VARIANT_STYLES: Record<NonNullable<ButtonProps["variant"]>, React.CSSProperties> = {
  primary: {
    background: "#89b4fa",
    color: "#1e1e2e",
    border: "none",
  },
  secondary: {
    background: "#313244",
    color: "#cdd6f4",
    border: "1px solid #45475a",
  },
  ghost: {
    background: "transparent",
    color: "#89b4fa",
    border: "1px solid #313244",
  },
  danger: {
    background: "#f38ba8",
    color: "#1e1e2e",
    border: "none",
  },
};

const SIZE_STYLES: Record<NonNullable<ButtonProps["size"]>, React.CSSProperties> = {
  sm: { padding: "2px 8px", fontSize: 11, borderRadius: 3 },
  md: { padding: "4px 12px", fontSize: 13, borderRadius: 4 },
  lg: { padding: "7px 18px", fontSize: 15, borderRadius: 6 },
};

export function Button({
  variant = "primary",
  size = "md",
  disabled = false,
  loading = false,
  icon,
  onClick,
  children,
  className,
}: ButtonProps) {
  const isDisabled = disabled || loading;

  const style: React.CSSProperties = {
    ...VARIANT_STYLES[variant],
    ...SIZE_STYLES[size],
    cursor: isDisabled ? "not-allowed" : "pointer",
    opacity: isDisabled ? 0.5 : 1,
    display: "inline-flex",
    alignItems: "center",
    gap: 6,
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
    fontWeight: 600,
    transition: "opacity 0.15s, background 0.15s",
    outline: "none",
    userSelect: "none",
  };

  return (
    <button
      style={style}
      disabled={isDisabled}
      onClick={isDisabled ? undefined : onClick}
      className={className}
    >
      {loading ? (
        <span style={{ display: "inline-block", width: "1em", height: "1em", border: "2px solid currentColor", borderTopColor: "transparent", borderRadius: "50%", animation: "spin 0.6s linear infinite" }} />
      ) : icon ? (
        <span style={{ display: "inline-flex", alignItems: "center", width: "1em", height: "1em" }}>{icon}</span>
      ) : null}
      {children}
    </button>
  );
}
