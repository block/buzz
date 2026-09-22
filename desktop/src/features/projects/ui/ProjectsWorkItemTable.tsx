import { useTranslation } from "@/i18n";

export const WORK_ITEM_TABLE_GRID_CLASS =
  "grid w-full grid-cols-[minmax(0,1fr)_5.5rem_4.5rem_3rem_5rem_1.5rem] items-center gap-x-3";

export function ProjectsWorkItemTableHeader({
  itemLabel,
  typeLabel,
}: {
  itemLabel: string;
  typeLabel: string;
}) {
  const { t } = useTranslation();
  return (
    <thead className="sr-only">
      <tr>
        <th scope="col">{itemLabel}</th>
        <th scope="col">{typeLabel}</th>
        <th scope="col">{t("projects.issue.status")}</th>
        <th scope="col">{t("projects.work-item-table.replies")}</th>
        <th scope="col">{t("projects.work-item-table.updated")}</th>
        <th scope="col">{t("projects.work-item-table.actions")}</th>
      </tr>
    </thead>
  );
}
