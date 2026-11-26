import { createTwoFilesPatch, diffArrays } from "diff";

export type MergeResult = { ok: true; text: string } | { ok: false };

interface Hunk {
  start: number;
  end: number;
  lines: string[];
}

/** Split text into lines keeping each line's own terminator (LF, CRLF, CR, or none). */
export function splitLines(text: string): string[] {
  if (!text) return [];
  return text.match(/[^\r\n]*(?:\r\n|\n|\r|$)/g)?.filter(Boolean) ?? [];
}

function extractHunks(base: string[], target: string[]): Hunk[] {
  const hunks: Hunk[] = [];
  let baseIndex = 0;
  let pending: Hunk | null = null;
  for (const part of diffArrays(base, target)) {
    if (part.added) {
      if (!pending) pending = { start: baseIndex, end: baseIndex, lines: [] };
      pending.lines.push(...part.value);
    } else if (part.removed) {
      if (!pending) pending = { start: baseIndex, end: baseIndex, lines: [] };
      baseIndex += part.value.length;
      pending.end = baseIndex;
    } else {
      if (pending) {
        hunks.push(pending);
        pending = null;
      }
      baseIndex += part.value.length;
    }
  }
  if (pending) hunks.push(pending);
  return hunks;
}

function sameLines(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((line, i) => line === b[i]);
}

/**
 * Conservative 3-way line merge. Combines disjoint edits cleanly while preserving
 * exact line terminators; refuses any overlapping edit or ambiguous same-line insertion.
 */
export function mergeThreeWay(
  base: string,
  theirs: string,
  ours: string,
): MergeResult {
  if (ours === theirs || ours === base) return { ok: true, text: theirs };
  if (theirs === base) return { ok: true, text: ours };

  const baseLines = splitLines(base);
  const theirHunks = extractHunks(baseLines, splitLines(theirs));
  const ourHunks = extractHunks(baseLines, splitLines(ours));

  const output: string[] = [];
  let cursor = 0;
  let ti = 0;
  let oi = 0;

  while (ti < theirHunks.length || oi < ourHunks.length) {
    const t = theirHunks[ti];
    const o = ourHunks[oi];

    if (!o) {
      output.push(...baseLines.slice(cursor, t.start), ...t.lines);
      cursor = t.end;
      ti++;
      continue;
    }
    if (!t) {
      output.push(...baseLines.slice(cursor, o.start), ...o.lines);
      cursor = o.end;
      oi++;
      continue;
    }

    if (t.start === o.start && t.end === o.end && sameLines(t.lines, o.lines)) {
      output.push(...baseLines.slice(cursor, t.start), ...t.lines);
      cursor = t.end;
      ti++;
      oi++;
      continue;
    }

    if (t.start === t.end && o.start === o.end && t.start === o.start) {
      return { ok: false };
    }

    if (t.end <= o.start) {
      output.push(...baseLines.slice(cursor, t.start), ...t.lines);
      cursor = t.end;
      ti++;
      continue;
    }
    if (o.end <= t.start) {
      output.push(...baseLines.slice(cursor, o.start), ...o.lines);
      cursor = o.end;
      oi++;
      continue;
    }

    return { ok: false };
  }

  output.push(...baseLines.slice(cursor));
  return { ok: true, text: output.join("") };
}

/** Unified patch suitable for the shared Diff component; empty when texts match. */
export function unifiedDiff(
  left: string,
  right: string,
  leftName = "latest",
  rightName = "draft",
): string {
  if (left === right) return "";
  return createTwoFilesPatch(leftName, rightName, left, right, "", "", {
    context: 3,
  });
}
