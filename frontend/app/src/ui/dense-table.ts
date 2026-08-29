import { append, el } from "./dom";

export type DenseTableColumn = {
  id: string;
  header: string;
  numeric?: boolean;
};

export type DenseTableCell = {
  text: string;
  numeric?: boolean;
};

export type DenseTableRow = {
  id: string;
  cells: Array<string | DenseTableCell>;
  unknown?: boolean;
  selected?: boolean;
};

export type DenseTableOptions = {
  columns: DenseTableColumn[];
  rows: DenseTableRow[];
  footer?: string;
  caption?: string;
  columnsTemplate?: string;
};

export function createDenseTable(options: DenseTableOptions): HTMLElement {
  const table = el("div", "vox-table");
  const template =
    options.columnsTemplate ?? `repeat(${Math.max(options.columns.length, 1)}, minmax(0, 1fr))`;

  const header = el("div", "vox-table__header");
  header.style.gridTemplateColumns = template;
  for (const column of options.columns) {
    const cell = el("span", column.numeric === true ? "vox-num" : undefined, column.header);
    append(header, cell);
  }
  append(table, header);

  for (const row of options.rows) {
    const line = el("div", "vox-table__row");
    line.style.gridTemplateColumns = template;
    if (row.unknown === true) line.classList.add("is-unknown");
    if (row.selected === true) line.classList.add("is-selected");
    for (const [index, cell] of row.cells.entries()) {
      const column = options.columns[index];
      const numeric =
        typeof cell === "string" ? column?.numeric === true : cell.numeric === true || column?.numeric === true;
      const text = typeof cell === "string" ? cell : cell.text;
      append(line, el("span", numeric ? "vox-num" : undefined, text));
    }
    append(table, line);
  }

  if (options.footer !== undefined) {
    const footer = el("div", "vox-table__footer");
    append(footer, el("span", undefined, options.footer));
    append(table, footer);
  }
  if (options.caption !== undefined) {
    append(table, el("div", "vox-table__caption", options.caption));
  }
  return table;
}
