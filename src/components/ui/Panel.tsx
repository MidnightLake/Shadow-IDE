import React, { useState } from "react";

interface PanelProps {
  title?: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  collapsible?: boolean;
  defaultCollapsed?: boolean;
}

export function Panel({
  title,
  actions,
  children,
  className,
  collapsible = false,
  defaultCollapsed = false,
}: PanelProps) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);

  const headerStyle: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    padding: "6px 10px",
    borderBottom: collapsed ? undefined : "1px solid #313244",
    background: "#181825",
    userSelect: "none",
    flexShrink: 0,
    gap: 6,
  };

  const panelStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: "column",
    background: "#1e1e2e",
    border: "1px solid #313244",
    borderRadius: 6,
    overflow: "hidden",
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
    color: "#cdd6f4",
  };

  return (
    <div style={panelStyle} className={className}>
      {(title || actions || collapsible) && (
        <div style={headerStyle}>
          {collapsible && (
            <button
              onClick={() => setCollapsed((p) => !p)}
              style={{
                background: "transparent",
                border: "none",
                color: "#6c7086",
                cursor: "pointer",
                fontSize: 10,
                padding: 0,
                lineHeight: 1,
              }}
              aria-expanded={!collapsed}
              aria-label={collapsed ? "Expand" : "Collapse"}
            >
              {collapsed ? "▶" : "▼"}
            </button>
          )}
          {title && (
            <span style={{ fontSize: 12, fontWeight: 700, color: "#89b4fa", flex: 1 }}>{title}</span>
          )}
          {actions && (
            <div style={{ display: "flex", alignItems: "center", gap: 4 }}>{actions}</div>
          )}
        </div>
      )}
      {!collapsed && (
        <div style={{ flex: 1, overflow: "auto" }}>
          {children}
        </div>
      )}
    </div>
  );
}
