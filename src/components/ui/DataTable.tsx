import * as ScrollArea from "@radix-ui/react-scroll-area";
import type { CSSProperties, ReactNode } from "react";

export interface DataTableColumn<T> {
  key: string;
  header: ReactNode;
  width?: CSSProperties["width"];
  align?: "left" | "center" | "right";
  className?: string;
  render: (row: T) => ReactNode;
}

export function DataTable<T>({ columns, rows, rowKey, empty }: {
  columns: DataTableColumn<T>[];
  rows: T[];
  rowKey: (row: T) => string | number;
  empty: ReactNode;
}) {
  return <ScrollArea.Root className="data-table-wrap">
    <ScrollArea.Viewport className="data-table-viewport">
      <table className="data-table">
        <colgroup>{columns.map(column => <col key={column.key} style={column.width ? { width: column.width } : undefined}/>)}</colgroup>
        <thead><tr>{columns.map(column => <th key={column.key} className={column.className} style={{ textAlign: column.align ?? "left" }}>{column.header}</th>)}</tr></thead>
        <tbody>{rows.map(row => <tr key={rowKey(row)}>{columns.map(column => <td key={column.key} className={column.className} style={{ textAlign: column.align ?? "left" }}>{column.render(row)}</td>)}</tr>)}</tbody>
      </table>
      {rows.length === 0 && <div className="data-table-empty">{empty}</div>}
    </ScrollArea.Viewport>
    <ScrollArea.Scrollbar className="data-table-scrollbar" orientation="vertical"><ScrollArea.Thumb className="data-table-thumb"/></ScrollArea.Scrollbar>
    <ScrollArea.Scrollbar className="data-table-scrollbar" orientation="horizontal"><ScrollArea.Thumb className="data-table-thumb"/></ScrollArea.Scrollbar>
    <ScrollArea.Corner className="data-table-corner"/>
  </ScrollArea.Root>;
}
