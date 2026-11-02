import {
  EditorSelection,
  EditorState,
  Prec,
  RangeSet,
  RangeValue,
  StateField,
  Transaction,
  type Extension,
  type Text,
  type TransactionSpec,
} from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import {
  editorPosition,
  openedLine,
  placedBlocks,
  present,
} from "./presentation";
import { setSource } from "./source";

/**
 * A condition whose markers have their own lines draws as a box around the lines between them,
 * and the marker lines are hidden. Each marker line and the line breaks on both sides of it form
 * an atomic edge, so the caret steps straight between the box and the text around it.
 *
 * Inside the box, an edit stays inside: Backspace and Delete stop at the edge. From outside, they
 * cross it, joining the neighbouring line as they would in plain text. A selection may span an
 * edge, but its edit leaves the marker of a box it only partly covers, so the box stays valid.
 *
 * Edges derive from the latest analysis and are mapped through edits until the next one arrives,
 * so these rules hold while that analysis is still on its way.
 */
class BoxEdge extends RangeValue {
  // Text inserted at either end of an edge stays out of it.
  startSide = 1;
  endSide = -1;
  constructor(
    readonly box: object,
    readonly side: "opening" | "closing",
  ) {
    super();
  }
}
type Edges = RangeSet<BoxEdge>;

export const edgeField = StateField.define<Edges>({
  create: () => RangeSet.empty,
  update(edges, tr) {
    let next = edges.map(tr.changes);
    for (const effect of tr.effects) {
      if (!effect.is(present)) continue;
      const doc = tr.state.doc;
      // Hidden frontmatter counts as the start of the file: no line above a box to step to.
      const firstLine = effect.value.frontmatter
        ? editorPosition(effect.value)(effect.value.frontmatter.to)
        : 0;
      const ranges = [];
      for (const { box } of placedBlocks(effect.value, doc)) {
        if (!box) continue;
        ranges.push(
          new BoxEdge(box, "opening").range(
            box.opening.from > firstLine ? box.opening.from - 1 : firstLine,
            box.opening.to + 1,
          ),
          new BoxEdge(box, "closing").range(
            box.closing.from - 1,
            Math.min(box.closing.to + 1, doc.length),
          ),
        );
      }
      next = RangeSet.of(ranges, true);
    }
    return next;
  },
  provide: (field) =>
    EditorView.atomicRanges.of((view) => view.state.field(field)),
});

/** A box at the start or end of the file has no line beyond this edge to step to. */
function sealed(doc: Text, from: number, to: number, edge: BoxEdge): boolean {
  return edge.side === "opening"
    ? doc.sliceString(from, from + 1) !== "\n"
    : doc.sliceString(to - 1, to) !== "\n";
}

interface Span {
  from: number;
  to: number;
  edge: BoxEdge;
}
function edgesTouching(edges: Edges, from: number, to: number): Span[] {
  const spans: Span[] = [];
  edges.between(from, to, (start, end, edge) => {
    if (start < to && end > from) spans.push({ from: start, to: end, edge });
  });
  return spans;
}

/** Backspace at the start of the line after a box joins that line onto the box's last line. */
function pullIn(doc: Text, edge: Span): TransactionSpec {
  const line = doc.lineAt(edge.to);
  return {
    changes: [
      { from: edge.from, insert: line.text },
      line.to < doc.length
        ? { from: line.from, to: line.to + 1 }
        : { from: line.from - 1, to: line.to },
    ],
    selection: EditorSelection.cursor(edge.from),
  };
}

/** Delete at the end of the line before a box joins the box's first line onto it. */
function pullOut(doc: Text, edges: Edges, edge: Span): TransactionSpec {
  const line = doc.lineAt(edge.to);
  let closing = edge.to;
  edges.between(edge.to, doc.length, (from, _to, other) => {
    if (other.box !== edge.edge.box || other.side !== "closing") return;
    closing = from;
    return false;
  });
  const onlyLine = line.to === closing;
  const selection = EditorSelection.cursor(edge.from);
  // Nothing left to pull: the empty box goes with this Delete.
  if (onlyLine && !line.length)
    return {
      changes: { from: edge.from, to: doc.lineAt(closing + 1).to },
      selection,
    };
  return {
    changes: [
      { from: edge.from, insert: line.text },
      { from: line.from, to: onlyLine ? line.to : line.to + 1 },
    ],
    selection,
  };
}

/** What Backspace or Delete does at an edge. Inside the box, that is nothing. */
function crossEdge(
  doc: Text,
  edges: Edges,
  edge: Span,
  head: number,
): TransactionSpec {
  if (edge.edge.side === "closing" && head === edge.to)
    return pullIn(doc, edge);
  if (edge.edge.side === "opening" && head === edge.from)
    return pullOut(doc, edges, edge);
  return {};
}

/** The same edit without the markers of any box it only partly covers. */
function keepMarkers(
  edges: Edges,
  from: number,
  to: number,
  insert: string,
): TransactionSpec | null {
  const touched = edgesTouching(edges, from, to);
  const kept = touched.filter(
    ({ edge }) =>
      !touched.some(
        (other) => other.edge.box === edge.box && other.edge.side !== edge.side,
      ),
  );
  if (!kept.length) return null;
  const changes: { from: number; to?: number; insert?: string }[] = [
    { from, insert },
  ];
  let position = from;
  for (const span of kept) {
    if (span.from > position) changes.push({ from: position, to: span.from });
    position = Math.max(position, span.to);
  }
  if (position < to) changes.push({ from: position, to });
  return {
    changes,
    selection: EditorSelection.cursor(from + insert.length),
  };
}

/**
 * Keys, typing and paste. Plain "input" is what Enter and its relatives use. Formatting wraps the
 * selected text, markers included, and uses its own event, and structured edits carry a source
 * snapshot, so neither is an edit of the visible text.
 */
const guardedEdit = /^(delete($|\.)|input$|input\.(type|paste)($|\.))/;
const guardEdits = EditorState.transactionFilter.of((tr) => {
  const edges = tr.startState.field(edgeField);
  const event = tr.annotation(Transaction.userEvent);
  if (
    !tr.docChanged ||
    !edges.size ||
    !event ||
    !guardedEdit.test(event) ||
    tr.effects.some((effect) => effect.is(setSource))
  )
    return tr;
  const changes: { from: number; to: number; insert: string }[] = [];
  tr.changes.iterChanges((from, to, _fromB, _toB, inserted) =>
    changes.push({ from, to, insert: inserted.toString() }),
  );
  // One selection range always makes one change.
  if (changes.length !== 1 || changes[0].from === changes[0].to) return tr;
  let { from, to } = changes[0];
  const doc = tr.startState.doc;
  const { main } = tr.startState.selection;
  let guarded: TransactionSpec | null = null;
  if (main.empty && event === "delete.line") {
    // deleteLine takes the break before a line. At an edge that break belongs to the marker,
    // so take the one after the line instead, unless it belongs to the next edge.
    const edge = edgesTouching(edges, from, from + 1)[0];
    if (edge) {
      from = edge.to;
      if (to < doc.length && !edgesTouching(edges, to, to + 1).length) to++;
    }
    guarded = keepMarkers(edges, from, to, "") ?? {
      changes: { from, to },
      selection: EditorSelection.cursor(from),
    };
  } else if (main.empty && event.startsWith("delete")) {
    const edge = edgesTouching(edges, from, to).find(
      (span) => span.from === from && span.to === to,
    );
    if (edge) guarded = crossEdge(doc, edges, edge, main.head);
  } else {
    guarded = keepMarkers(edges, from, to, changes[0].insert);
  }
  if (!guarded) return tr;
  return {
    ...guarded,
    effects: tr.effects,
    annotations: Transaction.userEvent.of(event),
    scrollIntoView: tr.scrollIntoView,
  };
});

/** Where a caret resting at `head` belongs: never on a hidden marker line. */
function restingPosition(state: EditorState, head: number): number {
  let resting = head;
  const doc = state.doc;
  state.field(edgeField).between(head, head, (from, to, edge) => {
    const hidden =
      (head > from && head < to) ||
      (edge.side === "opening" &&
        head === from &&
        sealed(doc, from, to, edge)) ||
      (edge.side === "closing" && head === to && sealed(doc, from, to, edge));
    if (!hidden) return;
    resting = edge.side === "opening" ? to : from;
    return false;
  });
  return resting;
}

// Selections keep their ends so Select All and friends still reach the markers.
const keepCaretVisible = EditorView.updateListener.of((update) => {
  const { main } = update.state.selection;
  if (
    !main.empty ||
    !(
      update.docChanged ||
      update.selectionSet ||
      update.transactions.some((tr) => tr.effects.some((e) => e.is(present)))
    )
  )
    return;
  const resting = restingPosition(update.state, main.head);
  if (resting !== main.head)
    update.view.dispatch({ selection: { anchor: resting } });
});

/** ArrowUp or ArrowDown out of a sealed box opens the line that is missing beyond it. */
function leaveSealedBox(forward: boolean) {
  return (view: EditorView): boolean => {
    const { state } = view;
    const { main } = state.selection;
    if (!main.empty) return false;
    const line = state.doc.lineAt(main.head);
    const boundary = forward ? line.to : line.from;
    const edge = edgesTouching(
      state.field(edgeField),
      boundary - 1,
      boundary + 1,
    ).find(
      ({ from, to, edge }) =>
        edge.side === (forward ? "closing" : "opening") &&
        (forward ? from : to) === boundary &&
        sealed(state.doc, from, to, edge),
    );
    if (!edge || view.moveToLineBoundary(main, forward).head !== boundary)
      return false;
    const position = forward ? edge.to : edge.from;
    view.dispatch({
      changes: { from: position, insert: "\n" },
      selection: { anchor: forward ? position + 1 : position },
      effects: openedLine.of(position),
      scrollIntoView: true,
      userEvent: "input",
    });
    return true;
  };
}

export const onlyBoxes: Extension = [
  edgeField,
  guardEdits,
  keepCaretVisible,
  Prec.high(
    keymap.of([
      { key: "ArrowUp", run: leaveSealedBox(false) },
      { key: "ArrowDown", run: leaveSealedBox(true) },
    ]),
  ),
];
