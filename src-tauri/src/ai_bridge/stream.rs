// TODO: extract streaming/SSE logic from chat.rs
// The SSE parsing (data: lines, delta.text extraction) and streaming loop
// inside ai_chat_with_tools are deeply intertwined with the tool-execution
// state machine. Extract once the chat loop is refactored into smaller units.
