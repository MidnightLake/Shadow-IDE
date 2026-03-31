import React from "react";

interface BadgeProps {
  variant?: "default" | "success" | "warning" | "error" | "info";
  size?: "sm" | "md";
  children: React.ReactNode;
  className?: string;
}

const VARIANT_COLORS: Record<NonNullable<BadgeProps["variant"]>, { bg: string; color: string }> = {
  default: { bg: "#313244", color: "#cdd6f4" },
  success: { bg: "#1e3a20", color: "#a6e3a1" },
  warning: { bg: "#3a2e1e", color: "#fab387" },
  error: { bg: "#3a1e1e", color: "#f38ba8" },
  info: { bg: "#1e2a3a", color: "#89b4fa" },
};

const SIZE_STYLES: Record<NonNullable<BadgeProps["size"]>, React.CSSProperties> = {
  sm: { fontSize: 9, padding: "1px 5px", borderRadius: 3 },
  md: { fontSize: 11, padding: "2px 7px", borderRadius: 4 },
};

export function Badge({ variant = "default", size = "md", children, className }: BadgeProps) {
  const { bg, color } = VARIANT_COLORS[variant];

  const style: React.CSSProperties = {
    ...SIZE_STYLES[size],
    background: bg,
    color,
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
    fontWeight: 600,
    display: "inline-flex",
    alignItems: "center",
    letterSpacing: "0.02em",
    userSelect: "none",
    whiteSpace: "nowrap",
  };

  return (
    <span style={style} className={className}>
      {children}
    </span>
  );
}
