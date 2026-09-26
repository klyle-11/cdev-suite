// Global typings for the cdev SDKs (Node: injected by `cdev run`; browser: /cdev.js).
declare global {
  const cdev: {
    /** Record a value (+ inferred type, source line) and return it unchanged. */
    w<T>(value: T, label?: string): T;
    watch<T>(value: T, label?: string): T;
    /** Wrap a function: records args (by param name), return value, errors, duration, caller. */
    trace<F extends (...args: any[]) => any>(fn: F, name?: string): F;
    /** Trace every method of a class (prototype) or object. */
    traceAll<T>(target: T, prefix?: string): T;
    log(...args: unknown[]): void;
    send(event: Record<string, unknown>): void;
    flush(): void;
  };
  interface Window { cdev: typeof cdev }
}
export {};
