import { useState, useEffect, useCallback } from "react";

interface UseResizeReturn {
  leftSidebarWidth: number;
  setLeftSidebarWidth: (w: number) => void;
  rightSidebarWidth: number;
  setRightSidebarWidth: (w: number) => void;
  terminalHeight: number;
  setTerminalHeight: (h: number) => void;
  resizingLeft: boolean;
  startResizingLeft: () => void;
  resizingRight: boolean;
  startResizingRight: () => void;
  resizingTerminal: boolean;
  startResizingTerminal: () => void;
}

export function useResize(
  initialLeftWidth: number,
  initialRightWidth: number,
  initialTerminalHeight: number,
): UseResizeReturn {
  const [leftSidebarWidth, setLeftSidebarWidth] = useState(initialLeftWidth);
  const [rightSidebarWidth, setRightSidebarWidth] = useState(initialRightWidth);
  const [terminalHeight, setTerminalHeight] = useState(initialTerminalHeight);
  const [resizingLeft, setResizingLeft] = useState(false);
  const [resizingRight, setResizingRight] = useState(false);
  const [resizingTerminal, setResizingTerminal] = useState(false);

  const startResizingLeft = useCallback(() => setResizingLeft(true), []);
  const startResizingRight = useCallback(() => setResizingRight(true), []);
  const startResizingTerminal = useCallback(() => setResizingTerminal(true), []);

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (resizingLeft) {
        const abW = 44;
        setLeftSidebarWidth(Math.max(150, Math.min(600, e.clientX - abW)));
      }
      if (resizingRight) {
        setRightSidebarWidth(Math.max(150, Math.min(600, window.innerWidth - e.clientX)));
      }
      if (resizingTerminal) {
        setTerminalHeight(Math.max(100, Math.min(500, window.innerHeight - e.clientY)));
      }
    };
    const handleMouseUp = () => {
      setResizingLeft(false);
      setResizingRight(false);
      setResizingTerminal(false);
    };
    if (resizingLeft || resizingRight || resizingTerminal) {
      document.addEventListener("mousemove", handleMouseMove);
      document.addEventListener("mouseup", handleMouseUp);
      document.body.style.userSelect = "none";
      document.body.style.cursor = (resizingLeft || resizingRight) ? "col-resize" : "row-resize";
    }
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [resizingLeft, resizingRight, resizingTerminal]);

  return {
    leftSidebarWidth, setLeftSidebarWidth,
    rightSidebarWidth, setRightSidebarWidth,
    terminalHeight, setTerminalHeight,
    resizingLeft, startResizingLeft,
    resizingRight, startResizingRight,
    resizingTerminal, startResizingTerminal,
  };
}
