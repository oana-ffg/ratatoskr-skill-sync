/** Thrown by `proceed` when a newer draft has replaced this one. Never shown to the user. */
class Superseded extends Error {}

/**
 * Debounce drafts, and run at most one load at a time.
 *
 * A load may make many backend calls, and typing can outpace them. Without this limit every pause
 * starts another load that nothing can stop, so they stack up and starve the whole app. A load
 * should call `proceed()` between backend calls, which abandons it once its draft is out of date.
 * The newest draft then runs when the current load ends, and any drafts in between are skipped.
 */
export function createLivePreview<Input, Output>(
  load: (input: Input, proceed: () => void) => Promise<Output>,
  result: (output: Output) => void,
  error: (error: unknown) => void,
  delay = 350,
): { schedule: (input: Input) => void; cancel: () => void } {
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let running = false;
  let waiting: { input: Input; request: number } | undefined;
  function start(input: Input, request: number): void {
    running = true;
    const proceed = () => {
      if (request !== generation) throw new Superseded();
    };
    void (async () => load(input, proceed))()
      .then(
        (output) => {
          if (request === generation) result(output);
        },
        (failure: unknown) => {
          if (request === generation) error(failure);
        },
      )
      .finally(() => {
        running = false;
        if (waiting) {
          const next = waiting;
          waiting = undefined;
          start(next.input, next.request);
        }
      });
  }
  function cancel(): void {
    generation++;
    clearTimeout(timer);
    waiting = undefined;
  }
  return {
    cancel,
    schedule(input): void {
      cancel();
      const request = generation;
      timer = setTimeout(() => {
        if (running) waiting = { input, request };
        else start(input, request);
      }, delay);
    },
  };
}
