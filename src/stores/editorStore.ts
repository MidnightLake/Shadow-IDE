import { useReducer, useCallback } from "react";

interface EditorState {
  activeFile: string | null;
  openFiles: string[];
  splitDirection: "horizontal" | "vertical" | null;
  ghostTextEnabled: boolean;
  blameEnabled: boolean;
  fontSize: number;
  fontFamily: string;
}

type EditorAction =
  | { type: "SET_ACTIVE_FILE"; payload: string | null }
  | { type: "OPEN_FILE"; payload: string }
  | { type: "CLOSE_FILE"; payload: string }
  | { type: "SET_SPLIT_DIRECTION"; payload: "horizontal" | "vertical" | null }
  | { type: "SET_GHOST_TEXT_ENABLED"; payload: boolean }
  | { type: "SET_BLAME_ENABLED"; payload: boolean }
  | { type: "SET_FONT_SIZE"; payload: number }
  | { type: "SET_FONT_FAMILY"; payload: string };

const initialState: EditorState = {
  activeFile: null,
  openFiles: [],
  splitDirection: null,
  ghostTextEnabled: false,
  blameEnabled: false,
  fontSize: 14,
  fontFamily: "JetBrains Mono, Fira Code, monospace",
};

function editorReducer(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case "SET_ACTIVE_FILE":
      return { ...state, activeFile: action.payload };
    case "OPEN_FILE":
      return {
        ...state,
        openFiles: state.openFiles.includes(action.payload)
          ? state.openFiles
          : [...state.openFiles, action.payload],
        activeFile: action.payload,
      };
    case "CLOSE_FILE": {
      const filtered = state.openFiles.filter((f) => f !== action.payload);
      const newActive =
        state.activeFile === action.payload
          ? (state.openFiles.find((f) => f !== action.payload) ?? null)
          : state.activeFile;
      return { ...state, openFiles: filtered, activeFile: newActive };
    }
    case "SET_SPLIT_DIRECTION":
      return { ...state, splitDirection: action.payload };
    case "SET_GHOST_TEXT_ENABLED":
      return { ...state, ghostTextEnabled: action.payload };
    case "SET_BLAME_ENABLED":
      return { ...state, blameEnabled: action.payload };
    case "SET_FONT_SIZE":
      return { ...state, fontSize: action.payload };
    case "SET_FONT_FAMILY":
      return { ...state, fontFamily: action.payload };
    default:
      return state;
  }
}

export function useEditorStore() {
  const [state, dispatch] = useReducer(editorReducer, initialState);

  const setActiveFile = useCallback((path: string | null) => dispatch({ type: "SET_ACTIVE_FILE", payload: path }), []);
  const openFile = useCallback((path: string) => dispatch({ type: "OPEN_FILE", payload: path }), []);
  const closeFile = useCallback((path: string) => dispatch({ type: "CLOSE_FILE", payload: path }), []);
  const setSplitDirection = useCallback((dir: "horizontal" | "vertical" | null) => dispatch({ type: "SET_SPLIT_DIRECTION", payload: dir }), []);
  const setGhostTextEnabled = useCallback((v: boolean) => dispatch({ type: "SET_GHOST_TEXT_ENABLED", payload: v }), []);
  const setBlameEnabled = useCallback((v: boolean) => dispatch({ type: "SET_BLAME_ENABLED", payload: v }), []);
  const setFontSize = useCallback((n: number) => dispatch({ type: "SET_FONT_SIZE", payload: n }), []);
  const setFontFamily = useCallback((f: string) => dispatch({ type: "SET_FONT_FAMILY", payload: f }), []);

  return {
    ...state,
    setActiveFile,
    openFile,
    closeFile,
    setSplitDirection,
    setGhostTextEnabled,
    setBlameEnabled,
    setFontSize,
    setFontFamily,
  };
}
