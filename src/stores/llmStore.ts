import { useReducer, useCallback } from "react";

type LlmProvider = "anthropic" | "ollama" | "lmstudio" | "llamacpp" | "vllm";

interface LlmState {
  provider: LlmProvider;
  model: string;
  ollamaAvailable: boolean;
  lmStudioAvailable: boolean;
  vllmAvailable: boolean;
}

type LlmAction =
  | { type: "SET_PROVIDER"; payload: LlmProvider }
  | { type: "SET_MODEL"; payload: string }
  | { type: "SET_AVAILABILITY"; payload: { key: "ollamaAvailable" | "lmStudioAvailable" | "vllmAvailable"; value: boolean } };

const initialState: LlmState = {
  provider: "anthropic",
  model: "claude-sonnet-4-6",
  ollamaAvailable: false,
  lmStudioAvailable: false,
  vllmAvailable: false,
};

function llmReducer(state: LlmState, action: LlmAction): LlmState {
  switch (action.type) {
    case "SET_PROVIDER":
      return { ...state, provider: action.payload };
    case "SET_MODEL":
      return { ...state, model: action.payload };
    case "SET_AVAILABILITY":
      return { ...state, [action.payload.key]: action.payload.value };
    default:
      return state;
  }
}

export function useLlmStore() {
  const [state, dispatch] = useReducer(llmReducer, initialState);

  const setProvider = useCallback((p: LlmProvider) => dispatch({ type: "SET_PROVIDER", payload: p }), []);
  const setModel = useCallback((m: string) => dispatch({ type: "SET_MODEL", payload: m }), []);
  const setAvailability = useCallback(
    (key: "ollamaAvailable" | "lmStudioAvailable" | "vllmAvailable", v: boolean) =>
      dispatch({ type: "SET_AVAILABILITY", payload: { key, value: v } }),
    []
  );

  return {
    ...state,
    setProvider,
    setModel,
    setAvailability,
  };
}
