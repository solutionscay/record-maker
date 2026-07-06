/// <reference types="svelte" />
/// <reference types="vite/client" />

// The shell (crates/server/templates/shell.html) defines one promise-based
// confirm dialog on `window`, shared by the whole app. Resolves true on OK.
interface Window {
  appConfirm?: (opts: {
    title?: string;
    message?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    danger?: boolean;
  }) => Promise<boolean>;
}
