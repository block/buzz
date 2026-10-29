export const WORK_ITEM_TABLE_GRID_CLASS =
  "grid w-full grid-cols-[minmax(0,1fr)_5.5rem_4.5rem_3rem_5rem_1.5rem] items-center gap-x-3";

export function ProjectsWorkItemTableHeader({
  itemLabel,
  typeLabel,
}: {
  itemLabel: string;
  typeLabel: string;
}) {
  return (
    <thead className="sr-only">
      <tr>
        <th scope="col">{itemLabel}</th>
        <th scope="col">{typeLabel}</th>
        <th scope="col">Status</th>
        <th scope="col">Replies</th>
        <th scope="col">Updated</th>
        <th scope="col">Actions</th>
      </tr>
    </thead>
  );
}
