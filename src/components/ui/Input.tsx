import React, { useId } from "react";

interface InputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  label?: string;
  error?: string;
  type?: "text" | "password" | "number" | "email";
  disabled?: boolean;
  icon?: React.ReactNode;
  className?: string;
}

export function Input({
  value,
  onChange,
  placeholder,
  label,
  error,
  type = "text",
  disabled = false,
  icon,
  className,
}: InputProps) {
  const id = useId();

  const containerStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: "column",
    gap: 4,
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  };

  const inputWrapStyle: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    background: "#181825",
    border: `1px solid ${error ? "#f38ba8" : "#313244"}`,
    borderRadius: 4,
    padding: "0 8px",
    gap: 6,
  };

  const inputStyle: React.CSSProperties = {
    flex: 1,
    background: "transparent",
    border: "none",
    outline: "none",
    color: disabled ? "#6c7086" : "#cdd6f4",
    padding: "5px 0",
    fontSize: 13,
    fontFamily: "inherit",
    cursor: disabled ? "not-allowed" : "text",
  };

  return (
    <div style={containerStyle} className={className}>
      {label && (
        <label htmlFor={id} style={{ fontSize: 11, color: "#6c7086", userSelect: "none" }}>
          {label}
        </label>
      )}
      <div style={inputWrapStyle}>
        {icon && (
          <span style={{ color: "#6c7086", display: "flex", alignItems: "center", flexShrink: 0 }}>
            {icon}
          </span>
        )}
        <input
          id={id}
          type={type}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          style={inputStyle}
        />
      </div>
      {error && (
        <span style={{ fontSize: 11, color: "#f38ba8" }}>{error}</span>
      )}
    </div>
  );
}
