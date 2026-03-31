import { useReducer, useCallback } from "react";

interface Message {
  role: "user" | "assistant";
  content: string;
  timestamp: number;
}

interface ChatState {
  messages: Message[];
  sessionId: string | null;
  isStreaming: boolean;
}

type ChatAction =
  | { type: "ADD_MESSAGE"; payload: Message }
  | { type: "SET_STREAMING"; payload: boolean }
  | { type: "SET_SESSION_ID"; payload: string | null }
  | { type: "CLEAR_MESSAGES" };

const initialState: ChatState = {
  messages: [],
  sessionId: null,
  isStreaming: false,
};

function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case "ADD_MESSAGE":
      return { ...state, messages: [...state.messages, action.payload] };
    case "SET_STREAMING":
      return { ...state, isStreaming: action.payload };
    case "SET_SESSION_ID":
      return { ...state, sessionId: action.payload };
    case "CLEAR_MESSAGES":
      return { ...state, messages: [] };
    default:
      return state;
  }
}

export function useChatStore() {
  const [state, dispatch] = useReducer(chatReducer, initialState);

  const addMessage = useCallback((msg: Message) => dispatch({ type: "ADD_MESSAGE", payload: msg }), []);
  const setStreaming = useCallback((v: boolean) => dispatch({ type: "SET_STREAMING", payload: v }), []);
  const setSessionId = useCallback((id: string | null) => dispatch({ type: "SET_SESSION_ID", payload: id }), []);
  const clearMessages = useCallback(() => dispatch({ type: "CLEAR_MESSAGES" }), []);

  return {
    ...state,
    addMessage,
    setStreaming,
    setSessionId,
    clearMessages,
  };
}
