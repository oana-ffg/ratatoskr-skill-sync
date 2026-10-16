import { diffWordsWithSpace, parsePatch } from "diff";

export interface Segment {
  text: string;
  changed: boolean;
}
export interface DiffLine {
  kind: "context" | "remove" | "add" | "missing";
  segments: Segment[];
}
export interface DiffRow {
  left: DiffLine;
  right: DiffLine;
  omitted?: number;
}

function line(text: string | undefined, kind: DiffLine["kind"]): DiffLine {
  return {
    kind: text === undefined ? "missing" : kind,
    segments: [
      { text: text ?? "No corresponding line", changed: kind !== "context" },
    ],
  };
}

// Unified diffs are supplied by Rust. This only aligns and highlights their text.
export function diffRows(diff: string, reverse = false): DiffRow[] {
  const rows: DiffRow[] = [];
  let previousEnd = 1;
  for (const patch of parsePatch(diff)) {
    for (const hunk of patch.hunks) {
      if (hunk.oldStart > previousEnd) {
        rows.push({
          left: line("", "context"),
          right: line("", "context"),
          omitted: hunk.oldStart - previousEnd,
        });
      }
      previousEnd = hunk.oldStart + hunk.oldLines;
      for (let i = 0; i < hunk.lines.length;) {
        const current = hunk.lines[i];
        if (current.startsWith(" ")) {
          rows.push({
            left: line(current.slice(1), "context"),
            right: line(current.slice(1), "context"),
          });
          i++;
        } else if (current.startsWith("-") || current.startsWith("+")) {
          const removed: string[] = [],
            added: string[] = [];
          while (i < hunk.lines.length && /^[+-]/.test(hunk.lines[i])) {
            const next = hunk.lines[i++];
            (next.startsWith("-") ? removed : added).push(next.slice(1));
          }
          for (let j = 0; j < Math.max(removed.length, added.length); j++) {
            const left = line(removed[j], "remove"),
              right = line(added[j], "add");
            if (removed[j] !== undefined && added[j] !== undefined) {
              const words = diffWordsWithSpace(removed[j], added[j]);
              left.segments = words
                .filter((word) => !word.added)
                .map((word) => ({ text: word.value, changed: word.removed }));
              right.segments = words
                .filter((word) => !word.removed)
                .map((word) => ({ text: word.value, changed: word.added }));
            }
            rows.push({ left, right });
          }
        } else i++; // Unified-diff metadata, including no-final-newline markers.
      }
    }
  }
  return reverse
    ? rows.map((row) => ({ ...row, left: row.right, right: row.left }))
    : rows;
}

export function collapseContext(rows: DiffRow[], expanded: boolean): DiffRow[] {
  if (expanded) return rows;
  const result: DiffRow[] = [];
  for (let i = 0; i < rows.length;) {
    let end = i;
    while (
      end < rows.length &&
      !rows[end].omitted &&
      rows[end].left.kind === "context"
    )
      end++;
    if (end - i > 6) {
      result.push(
        ...rows.slice(i, i + 3),
        {
          left: line("", "context"),
          right: line("", "context"),
          omitted: end - i - 6,
        },
        ...rows.slice(end - 3, end),
      );
    } else result.push(...rows.slice(i, end));
    if (end === i) {
      result.push(rows[i]);
      i++;
    } else i = end;
  }
  return result;
}
