/** Cache a dynamic import; a failed load is retried on next use instead of cached. */
export function lazy<T>(load: () => Promise<T>): () => Promise<T> {
  let module: Promise<T> | undefined;
  return () =>
    (module ??= load().catch((e: unknown) => {
      module = undefined;
      throw e;
    }));
}
