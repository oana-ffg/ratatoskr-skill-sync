import { Facet, StateEffect, StateField } from "@codemirror/state";
import { invertedEffects } from "@codemirror/commands";
/** CodeMirror positions count LF; disk ranges count every UTF-16 source unit.
 * Keep original line separators, including mixed endings, through incremental edits.
 */
export class SourceText {
  readonly breaks: number[] = [];
  readonly editorBreaks: number[] = [];
  readonly normalized: string;
  constructor(readonly source: string) {
    for (let i = 0; i < source.length - 1; i++)
      if (source[i] === "\r" && source[i + 1] === "\n") {
        this.editorBreaks.push(i - this.breaks.length);
        this.breaks.push(i);
      }
    this.normalized = source.replaceAll("\r\n", "\n");
  }
  private countBefore(items: number[], position: number) {
    let low = 0,
      high = items.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (items[mid] < position) low = mid + 1;
      else high = mid;
    }
    return low;
  }
  toEditor(position: number) {
    return position - this.countBefore(this.breaks, position);
  }
  toSource(position: number) {
    return position + this.countBefore(this.editorBreaks, position);
  }
  get newline() {
    return this.source.includes("\r\n") ? "\r\n" : "\n";
  }
}

const initialSource = Facet.define<string, string>({
  combine: (values) => values.at(-1) ?? "",
});
export const setSource = StateEffect.define<string>();
export const sourceField = StateField.define<SourceText>({
  create: (state) => new SourceText(state.facet(initialSource)),
  update(value, tr) {
    const explicit = tr.effects.find((effect) => effect.is(setSource));
    if (explicit?.is(setSource)) return new SourceText(explicit.value);
    if (!tr.docChanged) return value;
    let source = value.source;
    const changes: { from: number; to: number; insert: string }[] = [];
    tr.changes.iterChanges((from, to, _a, _b, insert) =>
      changes.push({
        from: value.toSource(from),
        to: value.toSource(to),
        insert: insert.sliceString(0, insert.length, value.newline),
      }),
    );
    for (const change of changes.reverse())
      source =
        source.slice(0, change.from) + change.insert + source.slice(change.to);
    return new SourceText(source);
  },
});
export function sourceExtension(source: string) {
  return [
    initialSource.of(source),
    sourceField,
    invertedEffects.of((tr) =>
      tr.docChanged
        ? [setSource.of(tr.startState.field(sourceField).source)]
        : [],
    ),
  ];
}
