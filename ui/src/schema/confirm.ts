// Route destructive schema actions through the shell's shared confirm dialog
// (window.appConfirm, defined in shell.html), falling back to native confirm if
// the island is ever mounted outside that shell so a delete is never silent.
export function confirmDanger(message: string, title = 'Confirm', confirmLabel = 'Delete'): Promise<boolean> {
  if (typeof window !== 'undefined' && window.appConfirm) {
    return window.appConfirm({ title, message, confirmLabel, danger: true });
  }
  return Promise.resolve(typeof window !== 'undefined' ? window.confirm(message) : true);
}
