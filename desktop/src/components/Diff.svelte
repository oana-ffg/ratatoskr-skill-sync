<script lang="ts">
  let {
    diff,
    left = "Your source",
    right = "Installed copy",
    reverse = false,
  }: {
    diff: string | null;
    left?: string;
    right?: string;
    reverse?: boolean;
  } = $props();
  // Presentation of a core-generated unified diff, never a template parser.
  const lines = $derived(
    (diff ?? "")
      .split("\n")
      .filter(
        (l) =>
          !l.startsWith("---") && !l.startsWith("+++") && !l.startsWith("@@"),
      ),
  );
</script>

{#if diff}<div class="diff-grid">
    <div>
      <h3>{left}</h3>
      <pre>{#each lines.filter((l) => !l.startsWith(reverse ? "-" : "+")) as line}<div
            class:diff-remove={line.startsWith("-")}
            class:diff-add={line.startsWith("+")}>{line.slice(1) ||
              " "}</div>{/each}</pre>
    </div>
    <div>
      <h3>{right}</h3>
      <pre>{#each lines.filter((l) => !l.startsWith(reverse ? "+" : "-")) as line}<div
            class:diff-add={line.startsWith("+")}
            class:diff-remove={line.startsWith("-")}>{line.slice(1) ||
              " "}</div>{/each}</pre>
    </div>
  </div>{:else}<p class="muted">
    Text comparison unavailable for binary or large files. Review the file
    details before applying.
  </p>{/if}
