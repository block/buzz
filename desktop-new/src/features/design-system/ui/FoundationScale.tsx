type FoundationItem = {
  token: string;
  value: string;
  use: string;
};

export function FoundationScale({ items }: { items: FoundationItem[] }) {
  return (
    <div className="flex flex-col">
      {items.map((item) => (
        <div
          key={item.token}
          className="grid gap-2 border-tertiary border-b py-3 last:border-b-0 sm:grid-cols-[10rem_7rem_1fr]"
        >
          <code className="text-mono text-primary">{item.token}</code>
          <span className="text-body-sm text-tertiary">{item.value}</span>
          <span className="text-body text-secondary">{item.use}</span>
        </div>
      ))}
    </div>
  );
}
