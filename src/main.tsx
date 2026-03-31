import { StrictMode, useState, useEffect, Component, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { invoke } from '@tauri-apps/api/core'
import { loader } from '@monaco-editor/react'

// Record first JS mark as early as possible
invoke('record_startup_mark', { markName: 'js-start', timestampMs: performance.now() }).catch(() => {})
import * as monaco from 'monaco-editor'
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker'
import jsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker'
import cssWorker from 'monaco-editor/esm/vs/language/css/css.worker?worker'
import htmlWorker from 'monaco-editor/esm/vs/language/html/html.worker?worker'
import tsWorker from 'monaco-editor/esm/vs/language/typescript/ts.worker?worker'
import './index.css'
import App from './App.tsx'
import MobileBridge from './MobileBridge.tsx'
import { ThemeProvider } from './contexts/ThemeContext'

// Configure Monaco to use locally bundled version instead of CDN
self.MonacoEnvironment = {
  getWorker(_, label) {
    if (label === 'json') return new jsonWorker()
    if (label === 'css' || label === 'scss' || label === 'less') return new cssWorker()
    if (label === 'html' || label === 'handlebars' || label === 'razor') return new htmlWorker()
    if (label === 'typescript' || label === 'javascript') return new tsWorker()
    return new editorWorker()
  },
}

loader.config({ monaco })

// Error boundary to catch render crashes
class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  render() {
    if (this.state.error) {
      return (
        <div style={{ background: '#1e1e2e', color: '#f38ba8', padding: 32, fontFamily: 'monospace', fontSize: 14, height: '100vh', overflow: 'auto' }}>
          <h2 style={{ color: '#cdd6f4' }}>ShadowIDE crashed during render</h2>
          <pre style={{ whiteSpace: 'pre-wrap', marginTop: 16 }}>{this.state.error.message}</pre>
          <pre style={{ whiteSpace: 'pre-wrap', marginTop: 8, color: '#a6adc8', fontSize: 12 }}>{this.state.error.stack}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}

const Root = () => {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    // Small delay to ensure Tauri environment is injected
    const timer = setTimeout(() => setReady(true), 100);
    return () => clearTimeout(timer);
  }, []);

  if (!ready) return <div style={{ background: '#0f172a', height: '100vh' }} />;

  // Tauri v2 detection: the real Tauri runtime defines __TAURI_INTERNALS__
  // as a non-writable, non-configurable property. Our HTML/JS mocks use
  // regular assignments which remain writable — so check the descriptor.
  const desc = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
  const isRealTauri = !!desc && !desc.writable && !desc.configurable;

  // If in real Tauri (desktop app), show App directly
  if (isRealTauri) {
    return <App />;
  }

  // Detect our iOS WKWebView: ContentView.swift registers "scanQR" and "trustCert" message handlers.
  const isOurWebView = !!(window as unknown as { webkit?: { messageHandlers?: Record<string, unknown> } }).webkit?.messageHandlers?.scanQR;

  // Also check standard mobile signals as fallback
  const isMobile = /iPhone|iPad|iPod|Android/i.test(navigator.userAgent)
    || (navigator.maxTouchPoints > 1 && /Mac/i.test(navigator.platform))
    || ('ontouchstart' in window && navigator.maxTouchPoints > 1 && !isRealTauri);

  // On our iOS app or any mobile device, show MobileBridge (connection screen)
  if (isOurWebView || isMobile) {
    return <MobileBridge />;
  }

  // Desktop browser (non-Tauri) — show App with mock
  return <App />;
};

const rootEl = document.getElementById('root')!
createRoot(rootEl).render(
  <StrictMode>
    <ErrorBoundary>
      <ThemeProvider>
        <Root />
      </ThemeProvider>
    </ErrorBoundary>
  </StrictMode>,
)
// Record react-mounted mark after root render call
invoke('record_startup_mark', { markName: 'react-mounted', timestampMs: performance.now() }).catch(() => {})
