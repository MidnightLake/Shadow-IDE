import { useState, useEffect } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";

export interface UpdateStatus {
  available: boolean;
  version: string;
  body: string;
  installing: boolean;
  error: string;
}

export function useUpdater() {
  const [status, setStatus] = useState<UpdateStatus>({
    available: false,
    version: "",
    body: "",
    installing: false,
    error: "",
  });
  const [update, setUpdate] = useState<Update | null>(null);

  useEffect(() => {
    // Check for updates shortly after launch
    const timer = setTimeout(async () => {
      try {
        const result = await check();
        if (result) {
          setUpdate(result);
          setStatus((s) => ({
            ...s,
            available: true,
            version: result.version,
            body: result.body ?? "",
          }));
        }
      } catch (e) {
        console.warn("Update check failed:", e);
      }
    }, 3000);

    return () => clearTimeout(timer);
  }, []);

  const installUpdate = async () => {
    if (!update) return;
    setStatus((s) => ({ ...s, installing: true, error: "" }));
    try {
      await update.downloadAndInstall();
    } catch (e) {
      setStatus((s) => ({
        ...s,
        installing: false,
        error: String(e),
      }));
    }
  };

  const dismiss = () => {
    setStatus((s) => ({ ...s, available: false }));
    setUpdate(null);
  };

  return { status, installUpdate, dismiss };
}
