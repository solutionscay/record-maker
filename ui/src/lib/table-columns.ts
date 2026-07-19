// Table-layout column projection (#117). A Table Browse column is backed by a
// top-level field object in a Body band (#154). The manager adds two optional,
// presentation-only values to that object's existing props bag:
//
//   tableColumn.visible — false hides the object from Table Browse/canvas
//   tableColumn.order   — explicit zero-based Browse order
//
// Older layouts carry neither key and retain their established geometry order
// (x, y, z, id). Once the manager changes order it writes an explicit order to
// every visible column, so subsequent reloads no longer depend on geometry.

import type { DesignModel, FieldChoice, ObjectView } from './model';
import { parseProps } from './object-props';

export interface TableColumnSettings {
  visible: boolean;
  order: number | null;
}

export interface TableFieldState {
  field: FieldChoice;
  /** Every top-level Body field object bound to this field, including hidden
   * duplicates. Mutations apply to the complete set so a duplicate cannot make
   * a supposedly hidden field reappear in Browse. */
  objectIds: number[];
  visibleObjectIds: number[];
  /** The object Browse will use after ordering + duplicate collapse. */
  primaryObjectId: number | null;
}

export interface TableColumnProjection {
  bodyPartId: number | null;
  visible: TableFieldState[];
  available: TableFieldState[];
}

export function tableColumnSettings(rawProps: string | null | undefined): TableColumnSettings {
  const bag = parseProps(rawProps).tableColumn;
  if (!bag || typeof bag !== 'object' || Array.isArray(bag)) {
    return { visible: true, order: null };
  }
  const value = bag as Record<string, unknown>;
  const rawOrder = value.order;
  return {
    visible: value.visible !== false,
    order:
      typeof rawOrder === 'number' && Number.isInteger(rawOrder) && rawOrder >= 0
        ? rawOrder
        : null,
  };
}

export function withTableColumnSettings(
  rawProps: string | null | undefined,
  patch: Partial<TableColumnSettings>,
): Record<string, unknown> {
  const props = parseProps(rawProps);
  const current = tableColumnSettings(rawProps);
  return {
    ...props,
    tableColumn: {
      visible: patch.visible ?? current.visible,
      ...(patch.order === undefined
        ? current.order === null
          ? {}
          : { order: current.order }
        : patch.order === null
          ? {}
          : { order: patch.order }),
    },
  };
}

/** Mirrors the server's Table Browse ordering. Explicitly ordered objects sort
 * before legacy/unconfigured objects; an entirely legacy layout therefore keeps
 * its historical geometry order byte-for-byte. */
export function compareTableColumnObjects(a: ObjectView, b: ObjectView): number {
  const ao = tableColumnSettings(a.props).order;
  const bo = tableColumnSettings(b.props).order;
  if (ao !== null || bo !== null) {
    if (ao === null) return 1;
    if (bo === null) return -1;
    if (ao !== bo) return ao - bo;
  }
  return a.x - b.x || a.y - b.y || a.z - b.z || a.id - b.id;
}

/** Split the primary table's schema fields into visible columns and available
 * fields. This is derived from the live EditorDoc render model, so create/hide/
 * reorder operations update the Inspector without a model refetch. */
export function projectTableColumns(model: DesignModel): TableColumnProjection {
  const bodyParts = model.parts.filter((part) => part.kind === 'body');
  const objects = bodyParts
    .flatMap((part) => part.objects)
    .filter(
      (object) =>
        object.kind === 'field' &&
        object.parentObjectId === undefined &&
        object.fieldId !== null,
    )
    .sort(compareTableColumnObjects);

  const byField = new Map<number, ObjectView[]>();
  for (const object of objects) {
    const fieldId = object.fieldId;
    if (fieldId === null) continue;
    const group = byField.get(fieldId);
    if (group) group.push(object);
    else byField.set(fieldId, [object]);
  }

  const visible: TableFieldState[] = [];
  const available: TableFieldState[] = [];
  for (const field of model.fields) {
    const group = byField.get(field.id) ?? [];
    const visibleObjects = group.filter((object) => tableColumnSettings(object.props).visible);
    const state: TableFieldState = {
      field,
      objectIds: group.map((object) => object.id),
      visibleObjectIds: visibleObjects.map((object) => object.id),
      primaryObjectId: visibleObjects[0]?.id ?? null,
    };
    if (visibleObjects.length > 0) visible.push(state);
    else available.push(state);
  }

  visible.sort((a, b) => {
    const ao = objects.find((object) => object.id === a.primaryObjectId);
    const bo = objects.find((object) => object.id === b.primaryObjectId);
    return ao && bo ? compareTableColumnObjects(ao, bo) : 0;
  });

  return {
    bodyPartId: bodyParts[0]?.id ?? null,
    visible,
    available,
  };
}
