// WebdriverIO config driving the Curio desktop head through tauri-driver.
//
// tauri-driver is a thin WebDriver proxy in front of the platform webview
// driver (WebKitWebDriver on Linux, msedgedriver for WebView2 on Windows). It
// exists on Linux + Windows only — macOS has no WebDriver for WKWebView, which
// is why macOS ships behind the manual release checklist
// (docs/release/macos-checklist.md) instead of an automated smoke.
//
// Run by the nightly `smoke` job (a hard gate), never by the per-PR loop. The
// harness owns everything the app touches: a throwaway CURIO_PROFILE, a
// throwaway destination directory and a 127.0.0.1 feed stub. The profile is
// seeded through the real `curio` CLI before the app boots — no test hooks in
// the app itself (D11).
import { type ChildProcess, execFile, spawn } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { type FeedStub, SEED_FEED_PATH, startFeedStub } from './support/stub';

// ESM module ("type": "module"): there is no __dirname.
const here = fileURLToPath(new URL('.', import.meta.url));
const repoRoot = resolve(here, '../../..');
const exe = process.platform === 'win32' ? '.exe' : '';

// src-tauri is a workspace member, so cargo emits to the workspace target dir
// (CARGO_TARGET_DIR when set, else <repo>/target). Override either binary
// with CURIO_BIN / CURIO_CLI_BIN; CURIO_PROFILE_BUILD=release picks release.
const targetDir = resolve(process.env.CARGO_TARGET_DIR ?? join(repoRoot, 'target'));
const profileBuild = process.env.CURIO_PROFILE_BUILD ?? 'debug';
const appBinary = process.env.CURIO_BIN ?? join(targetDir, profileBuild, `curio-desktop${exe}`);
const cliBinary = process.env.CURIO_CLI_BIN ?? join(targetDir, profileBuild, `curio${exe}`);

const DRIVER_PORT = 4444;
// Where failed scenarios leave a screenshot (gitignored by default).
const failureShots = resolve(process.env.CURIO_E2E_SHOTS ?? join(here, 'failures'));
const run = promisify(execFile);

let stub: FeedStub | undefined;
let scratch: string | undefined;
let tauriDriver: ChildProcess | undefined;

/** Runs one `curio` CLI command against the throwaway profile. */
async function curio(profile: string, ...args: string[]): Promise<void> {
    try {
        await run(cliBinary, ['--profile', profile, ...args]);
    } catch (error) {
        const { stdout = '', stderr = '' } = error as { stdout?: string; stderr?: string };
        throw new Error(`curio ${args.join(' ')} failed:\n${stdout}${stderr}`);
    }
}

/** Resolves once something accepts TCP connections on 127.0.0.1:port. */
async function waitForPort(port: number, timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
        const open = await new Promise<boolean>((done) => {
            const socket = connect(port, '127.0.0.1');
            socket.once('connect', () => {
                socket.destroy();
                done(true);
            });
            socket.once('error', () => done(false));
        });
        if (open) {
            return;
        }
        if (Date.now() > deadline) {
            throw new Error(`tauri-driver never listened on 127.0.0.1:${port}`);
        }
        await new Promise((wake) => setTimeout(wake, 100));
    }
}

export const config: WebdriverIO.Config = {
    runner: 'local',
    hostname: '127.0.0.1',
    port: DRIVER_PORT,
    framework: 'mocha',
    mochaOpts: { ui: 'bdd', timeout: 120_000 },
    specs: ['./specs/**/*.e2e.ts'],
    maxInstances: 1,
    tsConfigPath: './tsconfig.json',
    capabilities: [
        {
            'tauri:options': { application: appBinary },
        } as WebdriverIO.Capabilities,
    ],
    reporters: ['spec'],
    // A scenario that fails fails the run: no retries, no bail-outs hidden
    // behind a green exit code.
    specFileRetries: 0,

    // Launcher process, before any worker: start the stub and seed the
    // profile. The workers (and, through tauri-driver, the app) inherit
    // these env vars.
    onPrepare: async () => {
        scratch = mkdtempSync(join(tmpdir(), 'curio-e2e-'));
        const profile = join(scratch, 'profile');
        const destination = join(scratch, 'notes');
        stub = await startFeedStub(join(repoRoot, 'fixtures'));
        process.env.CURIO_PROFILE = profile;
        process.env.CURIO_E2E_STUB = stub.base;
        process.env.CURIO_E2E_DEST = destination;
        // W1: the CLI's explicit opt-in is the only way a loopback feed
        // becomes fetchable. Async exec (not spawnSync) so this process's
        // stub can answer the fetch while the CLI waits on it.
        await curio(profile, 'feed', 'add', `${stub.base}${SEED_FEED_PATH}`, '--allow-private-network');
        await curio(profile, 'dest', 'add', 'notes', destination);
        await curio(profile, 'fetch');
    },
    onComplete: async (exitCode: number) => {
        await stub?.close();
        if (scratch === undefined) {
            return;
        }
        // A red run keeps the app's own log next to the screenshots (the
        // nightly uploads both). A session that never starts produces no
        // screenshot, so the log — or its absence, meaning the app never
        // reached its setup hook — is the only clue to what went wrong.
        if (exitCode !== 0) {
            const logs = join(scratch, 'profile', 'logs');
            const keep = join(failureShots, 'app-logs');
            mkdirSync(failureShots, { recursive: true });
            if (existsSync(logs)) {
                cpSync(logs, keep, { recursive: true });
            } else {
                writeFileSync(
                    join(failureShots, 'app-logs-missing.txt'),
                    `no ${logs}: the app never reached its setup hook\n`,
                );
            }
        }
        rmSync(scratch, { recursive: true, force: true });
    },

    // Worker process: boot tauri-driver (installed by the nightly job,
    // version-pinned) for the session and tear it down after. On Windows the
    // job points TAURI_NATIVE_DRIVER at the runner's msedgedriver.
    beforeSession: async () => {
        const nativeDriver = process.env.TAURI_NATIVE_DRIVER;
        const args = nativeDriver ? ['--native-driver', nativeDriver] : [];
        tauriDriver = spawn('tauri-driver', args, {
            stdio: [null, process.stdout, process.stderr],
        });
        await waitForPort(DRIVER_PORT, 30_000);
    },
    afterSession: () => {
        tauriDriver?.kill();
    },

    // A failed scenario leaves a screenshot behind (the nightly job uploads
    // the directory as an artifact), so a red smoke is diagnosable.
    afterTest: async (test, _context, { passed }) => {
        if (passed) {
            return;
        }
        mkdirSync(failureShots, { recursive: true });
        const name = test.title.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
        await browser.saveScreenshot(join(failureShots, `${name}.png`));
    },
};
