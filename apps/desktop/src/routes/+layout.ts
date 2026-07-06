// Tauri webview app: no SSR, no prerender — adapter-static's SPA fallback
// serves the shell and the generated IPC bindings only work in the webview.
export const ssr = false;
export const prerender = false;

// DEV-ONLY: when running under `npm run dev` in a plain browser (no Tauri),
// install the in-browser IPC mock BEFORE the layout renders, so the stores'
// first fetches resolve against the preview dataset. A no-op in production
// and inside the packaged app (see install-mock.ts guards); the dynamic
// import keeps it out of the production bundle.
export async function load(): Promise<Record<string, never>> {
    if (import.meta.env.DEV) {
        const { installDevMock } = await import('$lib/dev/install-mock');
        await installDevMock();
    }
    return {};
}
