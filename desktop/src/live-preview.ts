/** Debounce drafts and ignore responses for superseded text or selections. */
export function createLivePreview<Input, Output>(
  load: (input: Input) => Promise<Output>,
  result: (output: Output) => void,
  error: (error: unknown) => void,
  delay = 350,
): { schedule: (input: Input) => void; cancel: () => void } {
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  function cancel(): void {
    generation++;
    clearTimeout(timer);
  }
  return {
    cancel,
    schedule(input): void {
      cancel();
      const request = generation;
      timer = setTimeout(() => {
        void load(input).then(
          (output) => {
            if (request === generation) result(output);
          },
          (failure: unknown) => {
            if (request === generation) error(failure);
          },
        );
      }, delay);
    },
  };
}
