export class RelayReconnectWaiters {
  private waiters = new Set<{
    resolve: () => void;
    reject: (error: Error) => void;
  }>();

  wait(): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      this.waiters.add({ resolve, reject });
    });
  }

  settle(error?: Error) {
    const waiters = [...this.waiters];
    this.waiters.clear();
    for (const waiter of waiters) {
      if (error) waiter.reject(error);
      else waiter.resolve();
    }
  }
}
