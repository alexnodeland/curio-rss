// WebdriverIO config driving the Curio desktop head through tauri-driver.
//
// tauri-driver is a thin WebDriver proxy in front of the platform webview
// driver (WebKitWebGTK on Linux, Edge WebView2 on Windows). It exists on
// Linux + Windows only — macOS has no WebDriver for WKWebView, which is why
// macOS ships behind the manual release checklist (docs/release/macos-checklist.md)
// instead of an automated smoke.
//
// This is a SKELETON: it is installed and run by the nightly `smoke` job, not
// by the per-PR gate, and several scenarios are `.skip` pending stable test
// selectors on the frontend. It never runs in the fast PR loop.
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { resolve } from 'node:path';

// The built binary under test. In the cargo workspace, artifacts land in the
// workspace-root target/, not a crate-local one. Override with CURIO_BIN.
const binary =
    process.env.CURIO_BIN ??
    resolve(
        __dirname,
        '../../../target',
        process.env.CURIO_PROFILE_BUILD ?? 'debug',
        process.platform === 'win32' ? 'curio-desktop.exe' : 'curio-desktop',
    );

let tauriDriver: ChildProcess;

export const config: WebdriverIO.Config = {
    runner: 'local',
    framework: 'mocha',
    mochaOpts: { ui: 'bdd', timeout: 60_000 },
    specs: ['./specs/**/*.e2e.ts'],
    maxInstances: 1,
    capabilities: [
        {
            // tauri-driver bridges this to the platform webview driver.
            browserName: 'wry',
            'tauri:options': { application: binary },
        } as WebdriverIO.Capabilities,
    ],
    reporters: ['spec'],
    autoCompileOpts: {
        autoCompile: true,
        tsNodeOpts: { transpileOnly: true, project: './tsconfig.json' },
    },

    // Boot tauri-driver on 127.0.0.1:4444 before the session, tear it down
    // after. `cargo install tauri-driver` provides the binary.
    onPrepare: () => {
        spawnSync('cargo', ['install', 'tauri-driver', '--locked'], { stdio: 'inherit' });
    },
    beforeSession: () => {
        tauriDriver = spawn('tauri-driver', [], { stdio: [null, process.stdout, process.stderr] });
    },
    afterSession: () => {
        tauriDriver?.kill();
    },
};
