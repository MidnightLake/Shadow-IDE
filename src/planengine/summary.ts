export interface PlanAuditTotals {
  done: number;
  partial: number;
  pending: number;
  total: number;
}

export interface PlanCriticalGap {
  index: string;
  gap: string;
  status: string;
  impact: string;
  effort: string;
}

export interface PlanengineShellSummary {
  auditStamp: string | null;
  totals: PlanAuditTotals | null;
  nextSteps: string[];
  criticalGaps: PlanCriticalGap[];
  visionHighlights: string[];
  firstLaunchSteps: string[];
  firstLaunchArtifacts: string[];
}

interface MarkdownSection {
  heading: string;
  body: string;
}

function normalizeHeading(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
}

function stripMarkdown(value: string): string {
  return value
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/~~([^~]+)~~/g, "$1")
    .replace(/^\d+[a-z]?\.\s*/, "")
    .replace(/^[-*]\s*/, "")
    .trim();
}

function parseMarkdownSections(markdown: string): MarkdownSection[] {
  const lines = markdown.replace(/\r/g, "").split("\n");
  const sections: MarkdownSection[] = [];
  let currentHeading = "Overview";
  let currentBody: string[] = [];

  const pushSection = () => {
    const body = currentBody.join("\n").trim();
    if (!currentHeading.trim() && !body) {
      return;
    }
    sections.push({
      heading: currentHeading.trim() || "Overview",
      body,
    });
  };

  for (const line of lines) {
    const match = line.match(/^#{1,6}\s+(.*)$/);
    if (match) {
      pushSection();
      currentHeading = match[1].trim();
      currentBody = [];
      continue;
    }
    currentBody.push(line);
  }

  pushSection();
  return sections;
}

function findSectionByHeading(sections: MarkdownSection[], heading: string): MarkdownSection | null {
  const query = normalizeHeading(heading);
  return sections.find((section) => normalizeHeading(section.heading).includes(query)) ?? null;
}

function extractListItems(body: string, limit?: number): string[] {
  const items = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => /^[-*]\s+/.test(line) || /^\d+[a-z]?\.\s+/.test(line))
    .map(stripMarkdown)
    .filter(Boolean);
  return typeof limit === "number" ? items.slice(0, limit) : items;
}

function extractPendingListItems(body: string, limit?: number): string[] {
  const items = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => /^[-*]\s+/.test(line) || /^\d+[a-z]?\.\s+/.test(line))
    .filter((line) => !(line.includes("✅") || line.includes("~~")))
    .map(stripMarkdown)
    .filter(Boolean);
  return typeof limit === "number" ? items.slice(0, limit) : items;
}

function extractNumberedLines(body: string, limit?: number): string[] {
  const items = body
    .split("\n")
    .map((line) => stripMarkdown(line.trim()))
    .filter((line) => /^\d+[a-z]?\.\s+/.test(line));
  return typeof limit === "number" ? items.slice(0, limit) : items;
}

function parseMarkdownTable(body: string): { headers: string[]; rows: string[][] } | null {
  const lines = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("|") && line.endsWith("|"));
  if (lines.length < 2) {
    return null;
  }
  const parseCells = (line: string) =>
    line
      .split("|")
      .slice(1, -1)
      .map((cell) => stripMarkdown(cell.trim()));
  const headers = parseCells(lines[0]);
  const rows = lines.slice(2).map(parseCells).filter((row) => row.length === headers.length);
  if (!headers.length || !rows.length) {
    return null;
  }
  return { headers, rows };
}

export function summarizePlanengineMarkdown(markdown: string): PlanengineShellSummary {
  if (!markdown.trim()) {
    return {
      auditStamp: null,
      totals: null,
      nextSteps: [],
      criticalGaps: [],
      visionHighlights: [],
      firstLaunchSteps: [],
      firstLaunchArtifacts: [],
    };
  }

  const sections = parseMarkdownSections(markdown);
  const visionSection = findSectionByHeading(sections, "Vision & North Star");
  const summarySection = findSectionByHeading(sections, "Summary");
  const nextStepsSection = findSectionByHeading(sections, "Recommended Next Steps");
  const criticalGapsSection = findSectionByHeading(sections, "Critical Gaps");
  const firstLaunchSection = findSectionByHeading(sections, "First-Launch Experience");
  const auditStampMatch = markdown.match(/\*\*Last audited:\*\*\s*([^\n]+)/i);

  const table = parseMarkdownTable(summarySection?.body ?? "");
  let totals: PlanAuditTotals | null = null;
  if (table) {
    const totalRow = table.rows.find((row) => normalizeHeading(row[0]).includes("total")) ?? table.rows[table.rows.length - 1];
    if (totalRow && totalRow.length >= 5) {
      const parseCount = (value: string) => {
        const parsed = Number.parseInt(value.replace(/[^0-9-]/g, ""), 10);
        return Number.isFinite(parsed) ? parsed : 0;
      };
      totals = {
        done: parseCount(totalRow[1]),
        partial: parseCount(totalRow[2]),
        pending: parseCount(totalRow[3]),
        total: parseCount(totalRow[4]),
      };
    }
  }

  const criticalGapsTable = parseMarkdownTable(criticalGapsSection?.body ?? "");
  const criticalGaps = (criticalGapsTable?.rows ?? [])
    .map((row, index) => {
      const [gapIndex, gap, status, impact, effort] = row;
      if (!gap) {
        return null;
      }
      const unresolved = !normalizeHeading(`${gap} ${status}`).includes("done") && !gap.includes("~~");
      if (!unresolved) {
        return null;
      }
      return {
        index: gapIndex || String(index + 1),
        gap,
        status: status || "Pending",
        impact: impact || "Impact not listed",
        effort: effort || "Effort not listed",
      };
    })
    .filter((gap): gap is PlanCriticalGap => Boolean(gap));

  return {
    auditStamp: auditStampMatch ? stripMarkdown(auditStampMatch[1]) : null,
    totals,
    nextSteps: extractPendingListItems(nextStepsSection?.body ?? "", 5),
    criticalGaps,
    visionHighlights: extractListItems(visionSection?.body ?? "", 5),
    firstLaunchSteps: extractNumberedLines(firstLaunchSection?.body ?? "", 7),
    firstLaunchArtifacts: extractListItems(firstLaunchSection?.body ?? "", 4),
  };
}
