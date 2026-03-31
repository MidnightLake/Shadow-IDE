import { useEffect, useRef, type RefObject } from "react";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

export function useFocusTrap(active: boolean): RefObject<HTMLDivElement | null> {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!active) return;

    const container = containerRef.current;
    if (!container) return;

    // Focus the first focusable element when trap activates
    const focusables = Array.from(
      container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
    ).filter((el) => !el.hasAttribute("disabled"));

    if (focusables.length > 0) {
      focusables[0].focus();
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;

      const currentFocusables = Array.from(
        container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      ).filter((el) => !el.hasAttribute("disabled"));

      if (currentFocusables.length === 0) return;

      const first = currentFocusables[0];
      const last = currentFocusables[currentFocusables.length - 1];
      const active = document.activeElement as HTMLElement;

      if (e.shiftKey) {
        // Shift+Tab: go backwards
        if (active === first || !container.contains(active)) {
          e.preventDefault();
          last.focus();
        }
      } else {
        // Tab: go forwards
        if (active === last || !container.contains(active)) {
          e.preventDefault();
          first.focus();
        }
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [active]);

  return containerRef;
}
