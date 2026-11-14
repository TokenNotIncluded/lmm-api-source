/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  type RowData,
  stockFeatures,
  tableFeatures,
  filterFns,
  sortFns,
  createCoreRowModel,
  type ReactTable as NativeTable,
  type TableOptions as NativeTableOptions,
  type ColumnDef as NativeColumnDef,
  type Column as NativeColumn,
  type Row as NativeRow,
  type Cell as NativeCell,
  type Header as NativeHeader,
} from '@tanstack/react-table'

import type { DataTableColumnMeta } from '@/tanstack-table'

// One feature contract for the shared table hook, column definitions and views.
export const dataTableFeatures = tableFeatures({
  ...stockFeatures,
  columnMeta: {} as DataTableColumnMeta,
  filterFns,
  sortFns,
  coreRowModel: createCoreRowModel(),
})
export type DataTableFeatures = typeof dataTableFeatures
export type Table<TData extends RowData> = NativeTable<DataTableFeatures, TData>
export type TableOptions<TData extends RowData> = NativeTableOptions<
  DataTableFeatures,
  TData
>
export type ColumnDef<
  TData extends RowData,
  TValue = unknown,
> = NativeColumnDef<DataTableFeatures, TData, TValue>
export type Column<TData extends RowData, TValue = unknown> = NativeColumn<
  DataTableFeatures,
  TData,
  TValue
>
export type Row<TData extends RowData> = NativeRow<DataTableFeatures, TData>
export type Cell<TData extends RowData, TValue = unknown> = NativeCell<
  DataTableFeatures,
  TData,
  TValue
>
export type Header<TData extends RowData, TValue = unknown> = NativeHeader<
  DataTableFeatures,
  TData,
  TValue
>
export * from '@tanstack/react-table'
