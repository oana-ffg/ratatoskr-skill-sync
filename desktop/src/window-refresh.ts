import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Native visibility is authoritative: hiding a WebView need not hide its DOM. */
export function watchWindow(
  activate: () => void,
  onerror: (error: unknown) => void,
  options: {
    intervalMs?: number;
    onVisibility?: (visible: boolean) => void;
  } = {},
): () => void {
  const { intervalMs = 60_000, onVisibility } = options;
  let disposed = false;
  let visible = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  let revision = 0;
  const stops: (() => void)[] = [];
  const native = isTauri() ? getCurrentWindow() : null;
  function track(promise: Promise<() => void>) {
    void promise
      .then((stop) => {
        if (disposed) stop();
        else stops.push(stop);
      })
      .catch((error) => {
        if (!disposed) onerror(error);
      });
  }
  function setVisible(value: boolean) {
    if (value !== visible) onVisibility?.(value);
    visible = value;
    if (!value) {
      if (timer) clearInterval(timer);
      timer = undefined;
    } else if (!timer) timer = setInterval(() => void check(true), intervalMs);
  }
  async function check(refresh: boolean) {
    const current = ++revision;
    try {
      const value = native
        ? (await native.isVisible()) && !(await native.isMinimized())
        : document.visibilityState !== "hidden";
      if (disposed || current !== revision) return;
      const shown = value && !visible;
      setVisible(value);
      if (value && (refresh || shown)) activate();
    } catch (error) {
      if (!disposed && current === revision) {
        setVisible(false);
        onerror(error);
      }
    }
  }
  if (native) {
    track(native.onFocusChanged(({ payload }) => void check(payload)));
    track(native.onResized(() => void check(false)));
    // The tray explicitly emits after show/unminimize, even if focus is unchanged.
    track(
      native.listen<boolean>("workspace-window-visible", ({ payload }) => {
        if (payload) void check(true);
        else {
          revision++;
          setVisible(false);
        }
      }),
    );
  } else {
    const focus = () => void check(true);
    const visibility = () => void check(false);
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", visibility);
    stops.push(() => {
      window.removeEventListener("focus", focus);
      document.removeEventListener("visibilitychange", visibility);
    });
  }
  void check(false);
  return () => {
    disposed = true;
    if (timer) clearInterval(timer);
    stops.forEach((stop) => stop());
  };
}
