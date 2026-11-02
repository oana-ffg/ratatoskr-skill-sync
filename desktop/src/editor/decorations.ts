import {
  StateField,
  type Extension,
  type Range,
  type Text,
} from "@codemirror/state";
import {
  Decoration,
  EditorView,
  WidgetType,
  type DecorationSet,
} from "@codemirror/view";
import { type Analysis, type Block, agentName, title } from "../api";
import { onlyBoxes } from "./only-box";
import {
  editorPosition,
  openedLine,
  placedBlocks,
  present,
  type Presentation,
} from "./presentation";
export function blockLabel(block: Block): string {
  return Object.entries(block.selectors)
    .map(([key, values]) =>
      values
        .map((v) => (key === "harness" ? agentName(v) : title(v)))
        .join(" or "),
    )
    .join(" · ");
}
function bandClass(block: Block): string {
  return `only-band ${blockDimension(block)} ${block.active ? "" : "inactive"}`;
}
class BlockHeader extends WidgetType {
  constructor(
    readonly block: Block,
    readonly action: Presentation["onblock"],
    readonly diagnostics: Analysis["diagnostics"] = [],
  ) {
    super();
  }
  toDOM(): HTMLElement {
    const row = document.createElement("span");
    row.className = `only-header ${blockDimension(this.block)}`;
    const label = document.createElement("span");
    label.textContent = `◆ Only for ${blockLabel(this.block)}`;
    row.append(label);
    for (const remove of [false, true]) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = remove ? "remove" : "edit";
      button.setAttribute(
        "aria-label",
        `${remove ? "Remove" : "Edit"} condition: ${blockLabel(this.block)}`,
      );
      button.onclick = () => this.action(this.block, remove);
      row.append(button);
    }
    return row;
  }
  ignoreEvent() {
    return true;
  }
}
/** The header of a box: a full-width block on the opening marker's hidden line. */
class BoxHeader extends BlockHeader {
  toDOM() {
    const band = document.createElement("div");
    band.className = bandClass(this.block);
    band.append(super.toDOM());
    for (const d of this.diagnostics)
      band.append(new Warning(d.message, d.severity).toDOM());
    return band;
  }
}
class VariablePill extends WidgetType {
  constructor(
    readonly name: string,
    readonly value: string | null,
  ) {
    super();
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "value-pill";
    span.textContent = `▣ ${title(this.name.split(".").at(-1)!)}`;
    span.title = this.value ?? "Unavailable on this machine";
    return span;
  }
}
class Warning extends WidgetType {
  constructor(
    readonly message: string,
    readonly severity: string,
  ) {
    super();
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = `inline-diagnostic ${this.severity}`;
    span.textContent = `${this.severity === "error" ? "●" : "⚠"} ${this.message}`;
    span.setAttribute("role", "note");
    return span;
  }
}
function blockDimension(block: Block): string {
  return block.selectors.harness?.includes("codex")
    ? "codex"
    : block.selectors.harness?.includes("claude")
      ? "claude"
      : block.selectors.os
        ? "os"
        : "machine";
}
function build(p: Presentation, doc: Text): DecorationSet {
  const viewLength = doc.length;
  if (p.raw) return Decoration.none;
  const at = editorPosition(p);
  const ranges: Range<Decoration>[] = [];
  const valid = (from: number, to: number) =>
    from >= 0 && at(to) <= viewLength && from < to;
  if (p.frontmatter && valid(p.frontmatter.from, p.frontmatter.to))
    ranges.push(
      Decoration.replace({
        block: true,
        atomic: true,
        inclusiveEnd: false,
      }).range(at(p.frontmatter.from), at(p.frontmatter.to)),
    );
  let diagnostics = p.analysis.diagnostics;
  for (const { block, opening, closing, box } of placedBlocks(p, doc)) {
    if (box) {
      // A diagnostic on a hidden marker has no text to sit beside, so the header carries it.
      const own = diagnostics.filter(
        (d) =>
          d.range.from >= block.opening.from && d.range.to <= block.opening.to,
      );
      diagnostics = diagnostics.filter((d) => !own.includes(d));
      ranges.push(
        Decoration.replace({
          widget: new BoxHeader(block, p.onblock, own),
          block: true,
        }).range(box.opening.from, box.opening.to),
        Decoration.replace({ block: true }).range(
          box.closing.from,
          box.closing.to,
        ),
      );
    } else {
      ranges.push(
        Decoration.replace({
          widget: new BlockHeader(block, p.onblock),
          atomic: true,
        }).range(opening.from, opening.to),
        Decoration.replace({ atomic: true }).range(closing.from, closing.to),
      );
    }
    const first = box
      ? box.opening.number + 1
      : doc.lineAt(opening.from).number;
    const last = box ? box.closing.number - 1 : doc.lineAt(closing.to).number;
    for (let number = first; number <= last; number++)
      ranges.push(
        Decoration.line({ class: bandClass(block) }).range(
          doc.line(number).from,
        ),
      );
  }
  for (const v of p.analysis.variables) {
    if (
      valid(v.range.from, v.range.to) &&
      !(p.frontmatter && v.range.from < p.frontmatter.to)
    )
      ranges.push(
        Decoration.replace({
          widget: new VariablePill(v.name, v.value),
          atomic: true,
        }).range(at(v.range.from), at(v.range.to)),
      );
  }
  for (const d of diagnostics) {
    if (at(d.range.to) <= viewLength)
      ranges.push(
        Decoration.widget({
          widget: new Warning(d.message, d.severity),
          side: 1,
        }).range(at(d.range.to)),
      );
  }
  return Decoration.set(ranges, true);
}
/**
 * Block decorations have to be inclusive, because CodeMirror draws an empty line beside any that
 * isn't. Mapping therefore pulls a line break opened right beside one into what it hides, and this
 * puts the break back outside.
 */
function releaseLine(set: DecorationSet, position: number): DecorationSet {
  const touching = (from: number, to: number, decoration: Decoration) =>
    decoration.spec.block && (from === position || to === position + 1);
  const released: Range<Decoration>[] = [];
  set.between(position, position + 1, (from, to, decoration) => {
    if (touching(from, to, decoration))
      released.push(
        decoration.range(
          from === position ? position + 1 : from,
          to === position + 1 ? position : to,
        ),
      );
  });
  return set.update({
    filter: (from, to, decoration) => !touching(from, to, decoration),
    add: released,
    sort: true,
  });
}
export const decorationField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    let next = value.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(present)) next = build(effect.value, tr.state.doc);
      if (effect.is(openedLine)) next = releaseLine(next, effect.value);
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});
export const structuredDecorations: Extension = [
  decorationField,
  onlyBoxes,
  EditorView.atomicRanges.of((view) =>
    view.state
      .field(decorationField)
      .update({ filter: (_from, _to, d) => d.spec.atomic === true }),
  ),
];
