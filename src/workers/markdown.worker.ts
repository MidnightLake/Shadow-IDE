// Web Worker — parses markdown to HTML off the main thread

self.onmessage = (e: MessageEvent<{ id: string; markdown: string }>) => {
  const { id, markdown } = e.data;
  self.postMessage({ id, html: convertMarkdown(markdown) });
};

function convertMarkdown(md: string): string {
  let html = md;

  // Escape HTML special chars
  html = html
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

  // Fenced code blocks
  html = html.replace(
    /```(\w*)\n([\s\S]*?)```/g,
    (_, lang: string, code: string) => {
      const langAttr = lang ? ` data-lang="${lang}"` : "";
      return `<pre class="md-code-block"${langAttr}><code class="md-code-inner">${code}</code></pre>`;
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

  // Unordered lists
  html = html.replace(/^[-*]\s+(.+)$/gm, "<li>$1</li>");
  html = html.replace(/(<li>[\s\S]+?<\/li>\n?)+/g, (match) => `<ul>${match}</ul>`);

  // Ordered lists
  html = html.replace(/^\d+\.\s+(.+)$/gm, "<oli>$1</oli>");
  html = html.replace(
    /(<oli>[\s\S]+?<\/oli>\n?)+/g,
    (match) => `<ol>${match.replace(/<oli>/g, "<li>").replace(/<\/oli>/g, "</li>")}</ol>`
  );

  // Links [text](url)
  html = html.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>'
  );

  // Bold **text**
  html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  // Italic *text*
  html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");
  // Inline code `code`
  html = html.replace(/`([^`]+)`/g, '<code class="md-inline-code">$1</code>');

  // Paragraphs
  html = html.replace(/^(?!<[a-z]|$)(.*\S.*)$/gm, "<p>$1</p>");

  return html;
}
