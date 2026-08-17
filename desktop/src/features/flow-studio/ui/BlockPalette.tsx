type FlowBlock = {
  block_type: string;
  name: string;
  description: string;
  category: string;
};

type BlockPaletteProps = {
  blocks: FlowBlock[];
};

export function BlockPalette({ blocks }: BlockPaletteProps) {
  return (
    <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {blocks.map((block) => (
        <li
          className="cursor-grab rounded-lg border border-border bg-card p-4 active:cursor-grabbing"
          draggable
          key={block.block_type}
          onDragStart={(event) => {
            event.dataTransfer.setData(
              "application/buzz-flow-block",
              JSON.stringify(block),
            );
          }}
        >
          <p className="text-sm font-medium">{block.name}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            {block.description}
          </p>
          <p className="mt-2 text-2xs uppercase tracking-wide text-muted-foreground">
            {block.category}
          </p>
        </li>
      ))}
    </ul>
  );
}

export type { FlowBlock };
