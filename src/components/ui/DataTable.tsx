import * as ScrollArea from "@radix-ui/react-scroll-area";
import { createColumnHelper, tableFeatures, useTable, type RowData } from "@tanstack/react-table";
import { useMemo, type CSSProperties, type ReactNode } from "react";

const features = tableFeatures({});

export interface DataTableColumn<T extends RowData> {
  key: string;
  header: ReactNode;
  width?: CSSProperties["width"];
  align?: "left" | "center" | "right";
  className?: string;
  render: (row: T) => ReactNode;
}

export function DataTable<T extends RowData>({ columns, rows, rowKey, empty }: {
  columns: DataTableColumn<T>[];
  rows: T[];
  rowKey: (row: T) => string | number;
  empty: ReactNode;
}) {
  const helper = createColumnHelper<typeof features, T>();
  const tableColumns = useMemo(() => helper.columns(columns.map(column => helper.display({
    id: column.key,
    header: () => column.header,
    cell: info => column.render(info.row.original),
  }))), [columns]);
  const table = useTable({ features, columns: tableColumns, data: rows, getRowId: row => String(rowKey(row)) });
  const columnById = new Map(columns.map(column => [column.key, column]));

  return <ScrollArea.Root className="data-table-wrap">
    <ScrollArea.Viewport className="data-table-viewport">
      <table className="data-table">
        <colgroup>{columns.map(column => <col key={column.key} style={column.width ? { width: column.width } : undefined}/>)}</colgroup>
        <thead>{table.getHeaderGroups().map(group => <tr key={group.id}>{group.headers.map(header => {
          const column = columnById.get(header.column.id);
          return <th key={header.id} className={column?.className} style={{ textAlign: column?.align ?? "left" }}>{header.isPlaceholder ? null : <table.FlexRender header={header}/>}</th>;
        })}</tr>)}</thead>
        <tbody>{table.getRowModel().rows.map(row => <tr key={row.id}>{row.getAllCells().map(cell => {
          const column = columnById.get(cell.column.id);
          return <td key={cell.id} className={column?.className} style={{ textAlign: column?.align ?? "left" }}><table.FlexRender cell={cell}/></td>;
        })}</tr>)}</tbody>
      </table>
      {table.getRowModel().rows.length === 0 && <div className="data-table-empty">{empty}</div>}
    </ScrollArea.Viewport>
    <ScrollArea.Scrollbar className="data-table-scrollbar" orientation="vertical"><ScrollArea.Thumb className="data-table-thumb"/></ScrollArea.Scrollbar>
    <ScrollArea.Scrollbar className="data-table-scrollbar" orientation="horizontal"><ScrollArea.Thumb className="data-table-thumb"/></ScrollArea.Scrollbar>
    <ScrollArea.Corner className="data-table-corner"/>
  </ScrollArea.Root>;
}
