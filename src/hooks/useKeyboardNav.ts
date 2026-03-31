import { useState, useCallback, type KeyboardEvent } from "react";

interface KeyboardNavOptions {
  wrap?: boolean;
  disabled?: boolean;
}

interface ItemProps {
  tabIndex: number;
  onKeyDown: (e: KeyboardEvent) => void;
  "data-focused": boolean;
}

interface ContainerProps {
  role: string;
  onKeyDown: (e: KeyboardEvent) => void;
}

interface KeyboardNavResult {
  focusedIndex: number;
  setFocusedIndex: (i: number) => void;
  getItemProps: (index: number) => ItemProps;
  containerProps: ContainerProps;
}

export function useKeyboardNav<T>(
  items: T[],
  onSelect: (item: T, index: number) => void,
  options?: KeyboardNavOptions,
): KeyboardNavResult {
  const wrap = options?.wrap ?? true;
  const disabled = options?.disabled ?? false;

  const [focusedIndex, setFocusedIndex] = useState(0);

  const navigate = useCallback(
    (e: KeyboardEvent) => {
      if (disabled || items.length === 0) return;

      switch (e.key) {
        case "ArrowDown": {
          e.preventDefault();
          setFocusedIndex((prev) => {
            if (prev >= items.length - 1) return wrap ? 0 : prev;
            return prev + 1;
          });
          break;
        }
        case "ArrowUp": {
          e.preventDefault();
          setFocusedIndex((prev) => {
            if (prev <= 0) return wrap ? items.length - 1 : prev;
            return prev - 1;
          });
          break;
        }
        case "Home": {
          e.preventDefault();
          setFocusedIndex(0);
          break;
        }
        case "End": {
          e.preventDefault();
          setFocusedIndex(items.length - 1);
          break;
        }
        case "Enter":
        case " ": {
          e.preventDefault();
          const item = items[focusedIndex];
          if (item !== undefined) onSelect(item, focusedIndex);
          break;
        }
      }
    },
    [disabled, items, focusedIndex, onSelect, wrap],
  );

  const getItemProps = useCallback(
    (index: number): ItemProps => ({
      tabIndex: index === focusedIndex ? 0 : -1,
      onKeyDown: navigate,
      "data-focused": index === focusedIndex,
    }),
    [focusedIndex, navigate],
  );

  const containerProps: ContainerProps = {
    role: "listbox",
    onKeyDown: navigate,
  };

  return {
    focusedIndex,
    setFocusedIndex,
    getItemProps,
    containerProps,
  };
}
