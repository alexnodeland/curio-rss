// ESLint covers what biome cannot: Svelte templates (eslint-plugin-svelte)
// and the raw-IPC import ban as an in-editor error. Biome remains the
// primary TS linter/formatter — the TS blocks here stay deliberately thin.
import tsParser from '@typescript-eslint/parser';
import svelte from 'eslint-plugin-svelte';
import { noRawText } from './eslint-rules/no-raw-text.js';
import svelteConfig from './svelte.config.js';

// The generated bindings file is the ONE module allowed to import
// @tauri-apps/api/core — everything else goes through its typed wrappers.
const rawInvokeBan = {
    'no-restricted-imports': [
        'error',
        {
            paths: [
                {
                    name: '@tauri-apps/api/core',
                    message:
                        'Hand-written invoke() is banned — import the generated wrappers from $lib/bindings instead.',
                },
            ],
        },
    ],
};

export default [
    {
        ignores: [
            'node_modules/',
            'build/',
            'coverage/',
            '.svelte-kit/',
            'src-tauri/',
            'e2e/',
            'src/lib/bindings.ts',
        ],
    },
    ...svelte.configs['flat/recommended'],
    {
        files: ['**/*.svelte'],
        languageOptions: {
            parserOptions: {
                parser: tsParser,
                extraFileExtensions: ['.svelte'],
                svelteConfig,
            },
        },
        rules: rawInvokeBan,
    },
    // Every word a user reads comes from the i18n catalogs (#155). Test
    // fixtures under src/tests are not shipped UI and stay out of scope.
    {
        files: ['src/lib/**/*.svelte', 'src/routes/**/*.svelte'],
        plugins: { curio: { rules: { 'no-raw-text': noRawText } } },
        rules: {
            'curio/no-raw-text': [
                'error',
                {
                    allow: [
                        // Reddit's own subreddit / user sigils (`r/rust`, `u/name`).
                        'r/',
                        'u/',
                        // The protocol name before a status code (`HTTP 304`).
                        'HTTP',
                        // Reddit's wordmark on the post header (aria-hidden branding).
                        'reddit',
                    ],
                },
            ],
        },
    },
    {
        files: ['**/*.ts', '**/*.svelte.ts'],
        languageOptions: {
            parser: tsParser,
        },
        rules: rawInvokeBan,
    },
];
