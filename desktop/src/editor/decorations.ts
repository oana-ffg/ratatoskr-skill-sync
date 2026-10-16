import { SourceText } from "./source";
import {
  StateEffect,
  StateField,
  type Extension,
  type Range,
} from "@codemirror/state";
import {
  Decoration,
  EditorView,
  WidgetType,
  type DecorationSet,
} from "@codemirror/view";
import { type Analysis, type Block, agentName, title } from "../api";
export interface Presentation {
  source?: string;
  analysis: Analysis;
  frontmatter: { from: number; to: number } | null;
  raw: boolean;
  onblock: (block: Block, remove: boolean) => void;
}
export const present = StateEffect.define<Presentation>();
export function blockLabel(block: Block): string {
  return Object.entries(block.selectors)
    .map(([key, values]) =>
      values
        .map((v) => (key === "harness" ? agentName(v) : title(v)))
        .join(" or "),
    )
    .join(" · ");
}
class BlockHeader extends WidgetType {
  constructor(
    readonly block: Block,
    readonly action: Presentation["onblock"],
  ) {
    super();
  }
  toDOM() {
    const row = document.createElement("span");
    row.className = `only-header ${this.block.selectors.harness?.includes("codex") ? "codex" : this.block.selectors.harness?.includes("claude") ? "claude" : this.block.selectors.os ? "os" : "machine"}`;
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
function build(p: Presentation, viewLength: number): DecorationSet {
  if (p.raw) return Decoration.none;
  const source = p.source === undefined ? null : new SourceText(p.source);
  const at = (pos: number) => source?.toEditor(pos) ?? pos;
  const ranges: Range<Decoration>[] = [];
  const valid = (from: number, to: number) =>
    from >= 0 && at(to) <= viewLength && from < to;
  if (p.frontmatter && valid(p.frontmatter.from, p.frontmatter.to))
    ranges.push(
      Decoration.replace({ atomic: true }).range(
        at(p.frontmatter.from),
        at(p.frontmatter.to),
      ),
    );
  for (const block of p.analysis.blocks) {
    if (!valid(block.range.from, block.range.to)) continue;
    ranges.push(
      Decoration.replace({
        widget: new BlockHeader(block, p.onblock),
        atomic: true,
      }).range(at(block.opening.from), at(block.opening.to)),
    );
    ranges.push(
      Decoration.replace({ atomic: true }).range(
        at(block.closing.from),
        at(block.closing.to),
      ),
    );
    ranges.push(
      Decoration.mark({
        class: `only-body ${block.active ? "" : "inactive"}`,
      }).range(at(block.body.from), at(block.body.to)),
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
  for (const d of p.analysis.diagnostics) {
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
export const decorationField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    let next = value.map(tr.changes);
    for (const effect of tr.effects)
      if (effect.is(present)) next = build(effect.value, tr.state.doc.length);
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});
export const structuredDecorations: Extension = [
  decorationField,
  EditorView.atomicRanges.of((view) =>
    view.state
      .field(decorationField)
      .update({ filter: (_from, _to, d) => d.spec.atomic === true }),
  ),
];
