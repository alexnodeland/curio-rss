// Tauri webview app: no SSR, no prerender — adapter-static's SPA fallback
// serves the shell and the generated IPC bindings only work in the webview.
export const ssr = false;
export const prerender = false;
