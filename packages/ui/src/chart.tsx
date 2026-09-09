import { useId } from "react";
/** One nonnegative scalar in a categorical bar chart. */
export interface ChartDatum {
  label: string;
  value: number;
}
/** A compact bar chart whose table is the accessible data source. Invalid values fail explicitly. */
export function BarChart({
  title,
  data,
  formatValue = String,
}: {
  title: string;
  data: readonly ChartDatum[];
  formatValue?: (value: number) => string;
}) {
  const id = useId();
  if (data.some((item) => !Number.isFinite(item.value) || item.value < 0))
    throw new RangeError("BarChart requires finite nonnegative values");
  if (new Set(data.map((item) => item.label)).size !== data.length)
    throw new Error("Chart labels must be unique");
  const max = Math.max(1, ...data.map((item) => item.value));
  return (
    <figure className="bui-stack" aria-labelledby={id}>
      <figcaption id={id} className="bui-label">
        {title}
      </figcaption>
      <table className="bui-chart">
        <thead className="bui-sr-only">
          <tr>
            <th scope="col">Category</th>
            <th scope="col">Value</th>
          </tr>
        </thead>
        <tbody>
          {data.map((item) => (
            <tr key={item.label}>
              <th scope="row">{item.label}</th>
              <td>
                <span
                  className="bui-chart-bar"
                  aria-hidden="true"
                  style={{ width: (item.value / max) * 100 + "%" }}
                />
                <span>{formatValue(item.value)}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  );
}
