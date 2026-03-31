import React, { useMemo, useEffect, useRef, useState } from "react";

interface MarkdownPreviewProps {
  content: string;
  filePath?: string;
}

/** Minimal markdown → HTML converter. No external deps. */
function markdownToHtml(md: string): string {
  let html = md;

  // Escape HTML special chars first (but we'll handle & carefully)
  // We do a minimal escape to avoid XSS from raw HTML in the markdown
  html = html
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

  // Fenced code blocks (``` lang\n...```)
  html = html.replace(
    /```(\w*)\n([\s\S]*?)```/g,
    (_, lang: string, code: string) => {
      const langAttr = lang ? ` data-lang="${lang}"` : "";
      return `<pre class="md-code-block"${langAttr}><code class="md-code-inner">${code}</code><button class="md-copy-btn" onclick="(function(btn){var pre=btn.parentElement;var code=pre.querySelector('code');navigator.clipboard&&navigator.clipboard.writeText(code.innerText).then(function(){btn.textContent='Copied!';setTimeout(function(){btn.textContent='Copy'},1500)});})(this)">Copy</button></pre>`;
    }
  );

  // Horizontal rules
  html = html.replace(/^---+$/gm, "<hr>");

  // Headers
  html = html.replace(/^#{6}\s+(.+)$/gm, "<h6>$1</h6>");
  html = html.replace(/^#{5}\s+(.+)$/gm, "<h5>$1</h5>");
  html = html.replace(/^#{4}\s+(.+)$/gm, "<h4>$1</h4>");
  html = html.replace(/^#{3}\s+(.+)$/gm, "<h3>$1</h3>");
  html = html.replace(/^#{2}\s+(.+)$/gm, "<h2>$1</h2>");
  html = html.replace(/^#{1}\s+(.+)$/gm, "<h1>$1</h1>");

  // Blockquotes
  html = html.replace(/^&gt;\s?(.*)$/gm, "<blockquote>$1</blockquote>");

  // Unordered lists (- item)
  html = html.replace(/^[-*]\s+(.+)$/gm, "<li>$1</li>");
  // Wrap consecutive <li> in <ul>
  html = html.replace(/(<li>[\s\S]+?<\/li>\n?)+/g, (match) => `<ul>${match}</ul>`);

  // Ordered lists (1. item)
  html = html.replace(/^\d+\.\s+(.+)$/gm, "<oli>$1</oli>");
  html = html.replace(/(<oli>[\s\S]+?<\/oli>\n?)+/g, (match) =>
    `<ol>${match.replace(/<oli>/g, "<li>").replace(/<\/oli>/g, "</li>")}</ol>`
  );

  // Images ![alt](src)
  html = html.replace(
    /!\[([^\]]*)\]\(([^)]+)\)/g,
    '<img alt="$1" src="$2" style="max-width:100%;border-radius:4px;">'
  );

  // Links [text](url)
  html = html.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer" style="color:#89b4fa;">$1</a>'
  );

  // Bold **text**
  html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  // Italic *text*
  html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");

  // Inline code `code`
  html = html.replace(/`([^`]+)`/g, '<code class="md-inline-code">$1</code>');

  // Math $...$
  html = html.replace(/\$([^$\n]+)\$/g, '<span class="md-math">$$$1$$</span>');

  // Paragraphs: wrap non-tag lines
  html = html.replace(/^(?!<[a-z]|$)(.*\S.*)$/gm, "<p>$1</p>");

  return html;
}

const PREVIEW_STYLES = `
  .md-preview {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    font-size: 14px;
    line-height: 1.7;
    color: #cdd6f4;
    padding: 20px 24px;
    max-width: 800px;
    margin: 0 auto;
  }
  .md-preview h1, .md-preview h2, .md-preview h3,
  .md-preview h4, .md-preview h5, .md-preview h6 {
    color: #cdd6f4;
    margin: 1.2em 0 0.5em;
    line-height: 1.3;
  }
  .md-preview h1 { font-size: 2em; border-bottom: 1px solid #313244; padding-bottom: 0.3em; }
  .md-preview h2 { font-size: 1.5em; border-bottom: 1px solid #313244; padding-bottom: 0.2em; }
  .md-preview h3 { font-size: 1.25em; }
  .md-preview p { margin: 0.7em 0; }
  .md-preview ul, .md-preview ol { padding-left: 1.8em; margin: 0.5em 0; }
  .md-preview li { margin: 0.25em 0; }
  .md-preview blockquote {
    border-left: 3px solid #89b4fa;
    margin: 0.8em 0;
    padding: 4px 12px;
    color: #a6adc8;
    background: #181825;
    border-radius: 0 4px 4px 0;
  }
  .md-preview hr {
    border: none;
    border-top: 1px solid #313244;
    margin: 1.5em 0;
  }
  .md-preview a { color: #89b4fa; }
  .md-preview img { max-width: 100%; border-radius: 4px; }
  .md-preview strong { color: #cdd6f4; font-weight: 700; }
  .md-inline-code {
    background: #313244;
    border-radius: 3px;
    padding: 1px 5px;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 0.9em;
    color: #f38ba8;
  }
  .md-code-block {
    background: #181825;
    border: 1px solid #313244;
    border-radius: 6px;
    padding: 14px 16px;
    overflow-x: auto;
    margin: 0.8em 0;
    position: relative;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 13px;
    line-height: 1.6;
  }
  .md-code-block[data-lang]::before {
    content: attr(data-lang);
    position: absolute;
    top: 6px;
    right: 48px;
    font-size: 10px;
    color: #6c7086;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .md-copy-btn {
    position: absolute;
    top: 6px;
    right: 8px;
    background: #313244;
    color: #cdd6f4;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 11px;
    cursor: pointer;
    font-family: inherit;
  }
  .md-copy-btn:hover { background: #45475a; }
  .md-code-inner { color: #a6e3a1; }
  .md-math {
    font-family: 'JetBrains Mono', monospace;
    color: #fab387;
    background: #1e1e2e;
    padding: 1px 4px;
    border-radius: 3px;
  }
`;

export default function MarkdownPreview({ content, filePath: _filePath }: MarkdownPreviewProps) {
  const [renderedHtml, setRenderedHtml] = useState(() => markdownToHtml(content));
  const workerRef = useRef<Worker | null>(null);

  // Use inline converter as fallback, and worker when available
  const html = useMemo(() => markdownToHtml(content), [content]);

  useEffect(() => {
    try {
      workerRef.current = new Worker(
        new URL("../workers/markdown.worker.ts", import.meta.url),
        { type: "module" }
      );
      workerRef.current.onmessage = (e: MessageEvent<{ id: string; html: string }>) => {
        setRenderedHtml(e.data.html);
      };
    } catch {
      // Worker not available (e.g. in test env), fallback to sync
    }
    return () => {
      workerRef.current?.terminate();
      workerRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (workerRef.current) {
      workerRef.current.postMessage({ id: "1", markdown: content });
    } else {
      setRenderedHtml(markdownToHtml(content));
    }
  }, [content]);

  // Inject styles once
  const styleInjected = React.useRef(false);
  if (!styleInjected.current) {
    const styleId = "md-preview-styles";
    if (!document.getElementById(styleId)) {
      const style = document.createElement("style");
      style.id = styleId;
      style.textContent = PREVIEW_STYLES;
      document.head.appendChild(style);
    }
    styleInjected.current = true;
  }

  // Handle copy button clicks (delegated via onclick in innerHTML)
  // The inline onclick in the generated HTML handles it directly.

  return (
    <div
      style={{
        height: "100%",
        overflowY: "auto",
        background: "#1e1e2e",
      }}
    >
      <div
        className="md-preview"
        // eslint-disable-next-line react/no-danger
        dangerouslySetInnerHTML={{ __html: renderedHtml || html }}
      />
    </div>
  );
}
