interface CollaborationPanelProps {
  visible?: boolean;
}

export default function CollaborationPanel({ visible = true }: CollaborationPanelProps) {
  if (!visible) return null;

  return (
    <div
      style={{
        position: "relative",
        height: "100%",
        background: "var(--bg-primary)",
        color: "var(--text-primary)",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          display: "flex",
          flexDirection: "column",
          gap: 8,
          padding: 14,
          color: "var(--text-muted)",
          fontSize: 12,
          pointerEvents: "none",
        }}
      >
        <div style={{ fontSize: 11, fontWeight: 700, color: "#7dd3fc", textTransform: "uppercase", letterSpacing: 0.7 }}>
          Collaboration
        </div>
        <div>Open a file and use this sidebar for presence, calls, and code review.</div>
      </div>
      <div
        id="collab-sidebar-root"
        style={{
          position: "absolute",
          inset: 0,
          overflowY: "auto",
        }}
      />
    </div>
  );
}
