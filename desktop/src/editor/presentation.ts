import { StateEffect, type Line, type Text } from "@codemirror/state";
import { type Analysis, type Block, type Range } from "../api";
import { SourceText } from "./source";

export interface Presentation {
  source?: string;
  analysis: Analysis;
  frontmatter: { from: number; to: number } | null;
  raw: boolean;
  onblock: (block: Block, remove: boolean) => void;
}
export const present = StateEffect.define<Presentation>();
/** A line break was just inserted at this position, right beside a box's hidden marker line. */
export const openedLine = StateEffect.define<number>();

/** Maps a core source offset to an editor position for this presentation. */
export function editorPosition(p: Presentation): (position: number) => number {
  const source = p.source === undefined ? null : new SourceText(p.source);
  return (position) => source?.toEditor(position) ?? position;
}

/** A block located in the editor document. */
export interface PlacedBlock {
  block: Block;
  opening: Range;
  closing: Range;
  /** The marker lines when this block draws as a box; null for inline markers. */
  box: { opening: Line; closing: Line } | null;
}

/** The line a marker has entirely to itself, matching how the core strips standalone markers. */
function markerLine(doc: Text, marker: Range): Line | null {
  const line = doc.lineAt(marker.from);
  return marker.to <= line.to &&
    !line.text.slice(0, marker.from - line.from).trim() &&
    !line.text.slice(marker.to - line.from).trim()
    ? line
    : null;
}

export function placedBlocks(p: Presentation, doc: Text): PlacedBlock[] {
  if (p.raw) return [];
  const at = editorPosition(p);
  // An analysis of an older draft can land just before its replacement. Hiding text or guarding
  // edits on the strength of it would act on the wrong characters, so its blocks wait.
  const isMarker = ({ from, to }: Range) =>
    doc.sliceString(at(from), at(to)).startsWith("<!--");
  return p.analysis.blocks
    .filter(
      ({ range, opening, closing }) =>
        range.from >= 0 &&
        range.from < range.to &&
        at(range.to) <= doc.length &&
        isMarker(opening) &&
        isMarker(closing),
    )
    .map((block) => {
      const opening = {
        from: at(block.opening.from),
        to: at(block.opening.to),
      };
      const closing = {
        from: at(block.closing.from),
        to: at(block.closing.to),
      };
      const open = markerLine(doc, opening);
      const close = markerLine(doc, closing);
      // Without a line between the markers there is nowhere inside to put the caret.
      const box =
        open && close && close.number > open.number + 1
          ? { opening: open, closing: close }
          : null;
      return { block, opening, closing, box };
    });
}
