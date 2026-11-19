import {
  SearchQuery,
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  replaceAll,
  replaceNext,
  selectMatches,
  setSearchQuery,
} from "@codemirror/search";
import type { EditorState } from "@codemirror/state";
import {
  runScopeHandlers,
  type EditorView,
  type Panel,
  type ViewUpdate,
} from "@codemirror/view";

type Match = { from: number; to: number };

/** CodeMirror's find-and-replace panel plus a browser-style match count in the find field. */
export function findPanel(view: EditorView): Panel {
  return new FindPanel(view);
}

class FindPanel implements Panel {
  readonly top = true;
  readonly dom: HTMLElement;
  private query: SearchQuery;
  private matches: Match[];
  private readonly searchField = textField("search", "Find");
  private readonly replaceField = textField("replace", "Replace");
  private readonly caseField = checkbox("case");
  private readonly reField = checkbox("re");
  private readonly wordField = checkbox("word");
  private readonly count = element("span", { className: "find-count" });

  constructor(private readonly view: EditorView) {
    this.query = getSearchQuery(view.state);
    this.showQuery();
    // CodeMirror focuses and selects this field when the panel opens or Mod-f repeats.
    this.searchField.setAttribute("main-field", "true");
    // The screen reader hears the new count whenever it changes.
    this.count.setAttribute("aria-live", "polite");
    const close = button("close", "×", () => closeSearchPanel(view));
    close.setAttribute("aria-label", "close");
    this.dom = element(
      "div",
      {
        className: "cm-search",
        // Every field and checkbox reports edits here, including pastes that no key press makes.
        oninput: () => this.commit(),
        onkeydown: (e) => this.keydown(e),
      },
      element(
        "span",
        { className: "find-field" },
        this.searchField,
        this.count,
      ),
      button("next", "next", () => findNext(view)),
      button("prev", "previous", () => findPrevious(view)),
      button("select", "all", () => selectMatches(view)),
      element("label", {}, this.caseField, "match case"),
      element("label", {}, this.reField, "regexp"),
      element("label", {}, this.wordField, "by word"),
      element("br"),
      this.replaceField,
      button("replace", "replace", () => replaceNext(view)),
      button("replaceAll", "replace all", () => replaceAll(view)),
      close,
    );
    this.matches = findMatches(view.state, this.query);
    this.showCount(view.state);
  }

  mount() {
    this.searchField.select();
  }

  update(update: ViewUpdate) {
    const query = getSearchQuery(update.state);
    const queryChanged = query !== getSearchQuery(update.startState);
    // A query set elsewhere, such as Mod-f over a selection, shows in the fields.
    if (!query.eq(this.query)) {
      this.query = query;
      this.showQuery();
    }
    // Counting walks the whole document, so only a new query or text recounts; moving between
    // matches only looks up which one is selected.
    if (queryChanged || update.docChanged)
      this.matches = findMatches(update.state, query);
    if (queryChanged || update.docChanged || update.selectionSet)
      this.showCount(update.state);
  }

  private commit() {
    const query = new SearchQuery({
      search: this.searchField.value,
      caseSensitive: this.caseField.checked,
      regexp: this.reField.checked,
      wholeWord: this.wordField.checked,
      replace: this.replaceField.value,
    });
    if (query.eq(this.query)) return;
    this.query = query;
    this.view.dispatch({ effects: setSearchQuery.of(query) });
  }

  // The editor's search-panel commands (Escape, Mod-g, …) only reach a panel that forwards them.
  private keydown(e: KeyboardEvent) {
    if (runScopeHandlers(this.view, e, "search-panel")) e.preventDefault();
    else if (e.key === "Enter" && e.target === this.searchField) {
      e.preventDefault();
      (e.shiftKey ? findPrevious : findNext)(this.view);
    } else if (e.key === "Enter" && e.target === this.replaceField) {
      e.preventDefault();
      replaceNext(this.view);
    }
  }

  private showQuery() {
    this.searchField.value = this.query.search;
    this.replaceField.value = this.query.replace;
    this.caseField.checked = this.query.caseSensitive;
    this.reField.checked = this.query.regexp;
    this.wordField.checked = this.query.wholeWord;
  }

  private showCount(state: EditorState) {
    this.count.textContent = countLabel(
      this.query,
      this.matches,
      state.selection.main,
    );
  }
}

function findMatches(state: EditorState, query: SearchQuery): Match[] {
  if (!query.valid) return [];
  const matches: Match[] = [];
  const cursor = query.getCursor(state);
  for (let next = cursor.next(); !next.done; next = cursor.next())
    matches.push({ from: next.value.from, to: next.value.to });
  return matches;
}

/** "3 of 12" once a match is selected, like a browser; until then only how many there are. */
function countLabel(
  query: SearchQuery,
  matches: readonly Match[],
  selection: Match,
): string {
  if (!query.search) return "";
  if (!query.valid) return "Invalid regexp";
  if (!matches.length) return "No matches";
  const current = matches.findIndex(
    (m) => m.from === selection.from && m.to === selection.to,
  );
  if (current >= 0) return `${current + 1} of ${matches.length}`;
  return matches.length === 1 ? "1 match" : `${matches.length} matches`;
}

function textField(name: string, label: string) {
  const field = element("input", {
    className: "cm-textfield",
    name,
    placeholder: label,
  });
  field.setAttribute("aria-label", label);
  return field;
}

function checkbox(name: string) {
  return element("input", { type: "checkbox", name });
}

function button(name: string, label: string, onclick: () => void) {
  return element(
    "button",
    { className: "cm-button", name, onclick, type: "button" },
    label,
  );
}

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}
