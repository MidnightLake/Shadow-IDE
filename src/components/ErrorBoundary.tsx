import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  fallback?: ReactNode;
  name?: string;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[ErrorBoundary${this.props.name ? ` ${this.props.name}` : ""}]`, error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) return this.props.fallback;
      return (
        <div style={{ padding: 16, color: "#ff6b6b", background: "#1a1a2e", borderRadius: 8, margin: 4, fontSize: 13 }}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>
            {this.props.name ? `${this.props.name} crashed` : "Component crashed"}
          </div>
          <div style={{ opacity: 0.7, fontFamily: "monospace", fontSize: 11, whiteSpace: "pre-wrap" }}>
            {this.state.error?.message}
          </div>
          <button
            onClick={() => this.setState({ hasError: false, error: null })}
            style={{ marginTop: 8, padding: "4px 12px", background: "#333", color: "#fff", border: "1px solid #555", borderRadius: 4, cursor: "pointer" }}
          >
            Retry
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
