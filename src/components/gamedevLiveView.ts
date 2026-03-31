export const GAMEDEV_LIVE_VIEW_PATH = "shadowide://gamedev/live-view";
export const GAMEDEV_LIVE_VIEW_NAME = "Live View";

export function isGameDevLiveViewPath(path?: string | null): boolean {
  return path === GAMEDEV_LIVE_VIEW_PATH;
}

export function isShadowIdeVirtualPath(path?: string | null): boolean {
  return Boolean(path?.startsWith("shadowide://"));
}
