const bus = new EventTarget();
export type UnlistenFn = () => void;
export type Event<T> = { event: string; id: number; payload: T };
export async function listen<T>(
  name: string,
  cb: (e: Event<T>) => void,
): Promise<UnlistenFn> {
  const h = (e: globalThis.Event) =>
    cb({ event: name, id: 0, payload: (e as CustomEvent).detail });
  bus.addEventListener(name, h);
  return () => bus.removeEventListener(name, h);
}
export async function once<T>(name: string, cb: (e: Event<T>) => void) {
  const un = await listen<T>(name, (e) => {
    un();
    cb(e);
  });
  return un;
}
export async function emit(name: string, payload?: unknown) {
  bus.dispatchEvent(new CustomEvent(name, { detail: payload }));
}
