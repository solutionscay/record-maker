// Thin fetch wrappers over the #149 `/layouts/*` endpoints, mirroring the
// schema builder's persist.ts. The component is the source of truth for
// what's on screen; these only talk to the server and return the views it
// assigns, so the UI can reflect server truth after every op.

import { getJson, postJson, postVoid } from '../shared/http';

export interface LayoutManagerView {
  id: number;
  name: string;
  tableId: number;
  tableName: string;
  view: string;
  position: number;
}

export interface TableOption {
  id: number;
  name: string;
}

export const listLayouts = (): Promise<LayoutManagerView[]> => getJson('/layouts/all');

export const listTables = (): Promise<TableOption[]> => getJson('/schema/tables');

export const createLayout = (
  name: string,
  tableId: number,
  view: string,
): Promise<LayoutManagerView> => postJson('/layouts', { name, tableId, view });

export const renameLayout = (id: number, name: string): Promise<LayoutManagerView> =>
  postJson(`/layouts/${id}/rename`, { name });

export const deleteLayout = (id: number): Promise<void> => postVoid(`/layouts/${id}/delete`);

export const reorderLayouts = (layoutIds: number[]): Promise<LayoutManagerView[]> =>
  postJson('/layouts/order', { layoutIds });
