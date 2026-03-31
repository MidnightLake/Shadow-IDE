import React, { type ReactNode } from "react";
import type { SidebarView, PanelZone } from "../types";
import { ErrorBoundary } from "../components/ErrorBoundary";

// TODO: extract to layout — renderPanelInner switch logic with all panel imports is tangled
// with App.tsx state (dispatch, rootPath, workspaceSettings etc.) and would require
// passing many props. The actual panel switching remains in App.tsx for now.

interface SidebarLayoutProps {
  leftView: SidebarView | null;
  rightView: SidebarView | null;
  panelZones: Record<SidebarView, PanelZone>;
  sidebarHidden: boolean;
  sidebarAutoHide: boolean;
  leftSidebarWidth: number;
  rightSidebarWidth: number;
  hasLeftSidebar: boolean;
  hasRightSidebar: boolean;
  onMouseEnterLeft: () => void;
  onMouseLeaveLeft: () => void;
  onMouseEnterRight: () => void;
  onMouseLeaveRight: () => void;
  onContextMenuLeft: (e: React.MouseEvent) => void;
  onContextMenuRight: (e: React.MouseEvent) => void;
  renderPanel: (view: SidebarView, isActive: boolean) => ReactNode;
}

export function SidebarLayout({
  leftView, rightView, panelZones,
  sidebarHidden, hasLeftSidebar, hasRightSidebar,
  onMouseEnterLeft, onMouseLeaveLeft,
  onMouseEnterRight, onMouseLeaveRight,
  onContextMenuLeft, onContextMenuRight,
  renderPanel,
}: SidebarLayoutProps) {
  return (
    <>
      {/* Left Sidebar */}
      <div
        className={`sidebar sidebar-left${sidebarHidden && hasLeftSidebar ? " auto-hidden" : ""}${!hasLeftSidebar ? " collapsed" : ""}`}
        onMouseEnter={onMouseEnterLeft}
        onMouseLeave={onMouseLeaveLeft}
        onContextMenu={onContextMenuLeft}
      >
        {(Object.entries(panelZones) as [SidebarView, PanelZone][])
          .filter(([, zone]) => zone === "left")
          .map(([view]) => (
            <div
              key={view}
              style={{
                display: leftView === view ? undefined : "none",
                height: "100%",
                overflowY: "auto",
                overflowX: "hidden",
              }}
            >
              <ErrorBoundary name={view}>{renderPanel(view, leftView === view || rightView === view)}</ErrorBoundary>
            </div>
          ))}
      </div>

      {/* Right Sidebar */}
      <div
        className={`sidebar sidebar-right${sidebarHidden && hasRightSidebar ? " auto-hidden" : ""}${!hasRightSidebar ? " collapsed" : ""}`}
        onMouseEnter={onMouseEnterRight}
        onMouseLeave={onMouseLeaveRight}
        onContextMenu={onContextMenuRight}
      >
        {(Object.entries(panelZones) as [SidebarView, PanelZone][])
          .filter(([, zone]) => zone === "right")
          .map(([view]) => (
            <div
              key={view}
              style={{
                display: rightView === view ? undefined : "none",
                height: "100%",
                overflowY: "auto",
                overflowX: "hidden",
              }}
            >
              <ErrorBoundary name={view}>{renderPanel(view, leftView === view || rightView === view)}</ErrorBoundary>
            </div>
          ))}
      </div>
    </>
  );
}
