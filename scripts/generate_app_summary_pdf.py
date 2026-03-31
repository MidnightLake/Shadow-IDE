from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import mm
from reportlab.platypus import (
    BaseDocTemplate,
    Frame,
    FrameBreak,
    PageTemplate,
    Paragraph,
    Spacer,
)


ROOT = Path(__file__).resolve().parents[1]
OUTPUT_DIR = ROOT / "output" / "pdf"
TMP_DIR = ROOT / "tmp" / "pdfs"
PDF_PATH = OUTPUT_DIR / "shadowide-app-summary.pdf"
PNG_PATH = TMP_DIR / "shadowide-app-summary-page-1.png"


ACCENT = colors.HexColor("#1B6C73")
ACCENT_DARK = colors.HexColor("#0E3E43")
TEXT = colors.HexColor("#1F2937")
MUTED = colors.HexColor("#5B6777")
LIGHT = colors.HexColor("#E7EFF1")


CONTENT = {
    "title": "ShadowIDE",
    "subtitle": "One-page repo summary generated from repository evidence only",
    "what_it_is": (
        "ShadowIDE is a lightweight, Rust-powered IDE built with Tauri v2. "
        "The repo also contains an in-repo native editor foundation under "
        "`native/` while keeping the current Tauri and React app intact."
    ),
    "who_its_for": (
        "Primary user/persona: Developers. Explicit persona statement: "
        "Not found in repo."
    ),
    "features": [
        "Tree-view file explorer with directory watching.",
        "Monaco-based code editor with syntax highlighting and multi-tab editing.",
        "Integrated terminal backed by xterm.js and portable-pty.",
        "AI chat, inline completion, and error explanation via OpenAI-compatible providers.",
        "Built-in tool calling loop plus token cache and truncation controls.",
        "TODO scanning, project state restore, recent projects, and workspace search.",
        "Remote access with TLS WebSocket pairing and generated QR codes.",
    ],
    "architecture": [
        "Frontend: React 19 + TypeScript + Vite app in `src/`; `src/main.tsx` loads Monaco workers and renders `App` or `MobileBridge`.",
        "Bridge: UI calls Rust commands through Tauri `invoke()` and exchanges app events with `listen()` / `emit()` in `src/App.tsx`.",
        "Backend: `src-tauri/src/lib.rs` registers services for files, terminal, AI bridge, project state, Git, diagnostics, plugins, remote server, RAG, LSP, and metrics.",
        "Integrations: Rust dependencies include `notify`, `portable-pty`, `reqwest`, `tokio-tungstenite`, `rusqlite`, and local Ferrum crates.",
        "Data flow: user action -> React panel -> Tauri command/event -> Rust service -> local OS, models, storage, or remote endpoint -> response/event back to UI.",
        "Native track: optional `native/` workspace hosts the planned editor foundation and runs separately from the current desktop app.",
    ],
    "run_steps": [
        "Install prerequisites from the repo README: Rust stable, Node.js 18+, and Tauri CLI.",
        "From the `shadow-ide` root, run `npm install`.",
        "Start the desktop app with `npm run dev` (package script) or `npm run tauri dev` (README command).",
    ],
    "sources": (
        "Sources used: README.md, package.json, src/main.tsx, src/App.tsx, "
        "src-tauri/Cargo.toml, src-tauri/src/lib.rs, native/README.md"
    ),
}


def build_styles():
    styles = getSampleStyleSheet()
    return {
        "section": ParagraphStyle(
            "Section",
            parent=styles["Heading2"],
            fontName="Helvetica-Bold",
            fontSize=10.8,
            leading=13,
            textColor=ACCENT_DARK,
            spaceBefore=0,
            spaceAfter=4,
        ),
        "body": ParagraphStyle(
            "Body",
            parent=styles["BodyText"],
            fontName="Helvetica",
            fontSize=8.45,
            leading=10.7,
            textColor=TEXT,
            spaceAfter=4,
        ),
        "bullet": ParagraphStyle(
            "Bullet",
            parent=styles["BodyText"],
            fontName="Helvetica",
            fontSize=8.05,
            leading=9.8,
            leftIndent=9,
            firstLineIndent=-6,
            bulletIndent=0,
            textColor=TEXT,
            spaceAfter=2,
        ),
        "footer": ParagraphStyle(
            "Footer",
            parent=styles["BodyText"],
            fontName="Helvetica",
            fontSize=6.6,
            leading=8,
            textColor=MUTED,
            alignment=1,
        ),
    }


def header(canvas, doc):
    width, height = A4
    margin = 14 * mm
    header_height = 30 * mm

    canvas.saveState()
    canvas.setFillColor(ACCENT)
    canvas.roundRect(
        margin,
        height - margin - header_height,
        width - (margin * 2),
        header_height,
        6,
        fill=1,
        stroke=0,
    )
    canvas.setFillColor(LIGHT)
    canvas.circle(width - margin - 18, height - margin - 16, 7, fill=1, stroke=0)
    canvas.circle(width - margin - 36, height - margin - 12, 3.5, fill=1, stroke=0)
    canvas.setFillColor(colors.white)
    canvas.setFont("Helvetica-Bold", 22)
    canvas.drawString(margin + 10, height - margin - 18, CONTENT["title"])
    canvas.setFont("Helvetica", 8.6)
    canvas.drawString(margin + 10, height - margin - 28, CONTENT["subtitle"])
    canvas.restoreState()


def section(title, body_text=None, bullets=None, styles=None):
    flow = [Paragraph(title, styles["section"])]
    if body_text:
        flow.append(Paragraph(body_text, styles["body"]))
    if bullets:
        for item in bullets:
            flow.append(Paragraph(item, styles["bullet"], bulletText="-"))
    return flow


def build_story(styles):
    story = [
        Spacer(1, 1.5 * mm),
    ]

    story.extend(section("What It Is", CONTENT["what_it_is"], styles=styles))
    story.extend(section("Who It's For", CONTENT["who_its_for"], styles=styles))
    story.extend(section("What It Does", bullets=CONTENT["features"], styles=styles))
    story.append(FrameBreak())
    story.extend(section("How It Works", bullets=CONTENT["architecture"], styles=styles))
    story.extend(section("How To Run", bullets=CONTENT["run_steps"], styles=styles))
    story.append(Spacer(1, 3 * mm))
    story.append(Paragraph(CONTENT["sources"], styles["footer"]))
    return story


def generate_pdf():
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    TMP_DIR.mkdir(parents=True, exist_ok=True)

    page_width, page_height = A4
    margin = 14 * mm
    gutter = 6 * mm
    header_height = 34 * mm
    footer_height = 12 * mm
    frame_top = page_height - margin - header_height - 4 * mm
    frame_bottom = margin + footer_height
    frame_height = frame_top - frame_bottom
    column_width = (page_width - (margin * 2) - gutter) / 2

    left = Frame(
        margin,
        frame_bottom,
        column_width,
        frame_height,
        leftPadding=0,
        rightPadding=0,
        topPadding=0,
        bottomPadding=0,
        id="left",
    )
    right = Frame(
        margin + column_width + gutter,
        frame_bottom,
        column_width,
        frame_height,
        leftPadding=0,
        rightPadding=0,
        topPadding=0,
        bottomPadding=0,
        id="right",
    )

    doc = BaseDocTemplate(
        str(PDF_PATH),
        pagesize=A4,
        leftMargin=margin,
        rightMargin=margin,
        topMargin=margin,
        bottomMargin=margin,
        title="ShadowIDE App Summary",
        author="OpenAI Codex",
    )
    doc.addPageTemplates(PageTemplate(id="summary", frames=[left, right], onPage=header))
    doc.build(build_story(build_styles()))


def render_preview():
    import fitz

    doc = fitz.open(PDF_PATH)
    page = doc.load_page(0)
    pix = page.get_pixmap(matrix=fitz.Matrix(2, 2), alpha=False)
    pix.save(PNG_PATH)
    doc.close()


if __name__ == "__main__":
    generate_pdf()
    render_preview()
    print(PDF_PATH)
    print(PNG_PATH)
