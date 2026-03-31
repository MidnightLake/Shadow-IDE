import type { OpenFile } from "./Editor";
import type { PlanengineShellSummary } from "../planengine/summary";

interface TitleBarProps {
  rootPath: string;
  activeFile: OpenFile | undefined;
  isMobileDevice: boolean;
  appWindow: { minimize: () => void; toggleMaximize: () => void; close: () => void } | null;
  planSummary?: PlanengineShellSummary;
  planLaunchpadVisible?: boolean;
  onOpenPlanengine?: () => void;
  onOpenGamePanel?: () => void;
  onOpenLiveView?: () => void;
}

function getLastPathSegment(path: string): string {
  if (!path) {
    return "/";
  }
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  if (!normalized) {
    return "/";
  }
  return normalized.split("/").pop() || "/";
}

export function TitleBar({
  rootPath,
  activeFile,
  isMobileDevice,
  appWindow,
  planSummary,
  planLaunchpadVisible = false,
  onOpenPlanengine,
  onOpenGamePanel,
  onOpenLiveView,
}: TitleBarProps) {
  const projectName = getLastPathSegment(rootPath);
  const progressLabel = planSummary?.totals ? `${planSummary.totals.done}/${planSummary.totals.total}` : null;
  const blockerLabel = planSummary ? `${planSummary.criticalGaps.length} gap${planSummary.criticalGaps.length === 1 ? "" : "s"}` : null;
  const planTitle = planSummary
    ? `PlanEngine${progressLabel ? ` ${progressLabel}` : ""}${blockerLabel ? ` • ${blockerLabel}` : ""}${planSummary.nextSteps[0] ? ` • Next: ${planSummary.nextSteps[0]}` : ""}`
    : "Open PlanEngine roadmap";
  const centerTitle = projectName === "/" ? "ShadowIDE v0.84.0" : `ShadowIDE v0.84.0 • ${projectName}`;

  return (
    <div className="title-bar" data-tauri-drag-region>
      <div className="title-bar-left" data-tauri-drag-region>
        <span className="title-bar-project" title={rootPath} data-tauri-drag-region>{projectName}</span>
        {activeFile && (
          <>
            <span className="title-bar-sep" data-tauri-drag-region>/</span>
            <span className="title-bar-file" data-tauri-drag-region>{activeFile.name}</span>
            {activeFile.modified && <span className="title-bar-modified">M</span>}
          </>
        )}
      </div>

      <div className="title-bar-center" data-tauri-drag-region title={centerTitle}>
        <span className="title-bar-app-name" data-tauri-drag-region>ShadowIDE</span>
        <span className="title-bar-version" data-tauri-drag-region>v0.84.0</span>
        {projectName !== "/" && (
          <>
            <span className="title-bar-center-dot" data-tauri-drag-region>•</span>
            <span className="title-bar-center-project" data-tauri-drag-region>{projectName}</span>
          </>
        )}
      </div>

      <div className="title-bar-right">
        {!isMobileDevice && (onOpenPlanengine || onOpenGamePanel || onOpenLiveView) && (
          <div className="title-bar-shortcuts">
            {planSummary && (
              <div className="title-bar-plan-pulse" title={planTitle}>
                {progressLabel && <span className="title-bar-plan-progress">{progressLabel}</span>}
                {blockerLabel && <span className="title-bar-plan-gap">{blockerLabel}</span>}
              </div>
            )}
            {onOpenPlanengine && (
              <button
                className={`title-bar-shortcut title-bar-shortcut-plan${planLaunchpadVisible ? " title-bar-shortcut-active" : ""}`}
                onClick={onOpenPlanengine}
                title={planLaunchpadVisible ? "Hide PlanEngine launchpad" : "Show PlanEngine launchpad"}
              >
                Plan
              </button>
            )}
            {onOpenGamePanel && (
              <button className="title-bar-shortcut" onClick={onOpenGamePanel} title="Open ShadowEditor game workflow">
                Game
              </button>
            )}
            {onOpenLiveView && (
              <button className="title-bar-shortcut" onClick={onOpenLiveView} title="Open the live viewport">
                View
              </button>
            )}
          </div>
        )}

        {!isMobileDevice && appWindow && (
          <div className="title-bar-controls">
            <button className="title-bar-btn" onClick={() => appWindow.minimize()} title="Minimize">
              <svg viewBox="0 0 12 12"><line x1="2" y1="6" x2="10" y2="6" stroke="currentColor" strokeWidth="1.2"/></svg>
            </button>
            <button className="title-bar-btn" onClick={() => appWindow.toggleMaximize()} title="Maximize">
              <svg viewBox="0 0 12 12"><rect x="2" y="2" width="8" height="8" rx="1" stroke="currentColor" strokeWidth="1.2" fill="none"/></svg>
            </button>
            <button className="title-bar-btn title-bar-close" onClick={() => appWindow.close()} title="Close">
              <svg viewBox="0 0 12 12"><line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" strokeWidth="1.2"/><line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" strokeWidth="1.2"/></svg>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
