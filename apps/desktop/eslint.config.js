// ESLint covers what biome cannot: Svelte templates (eslint-plugin-svelte)
// and the raw-IPC import ban as an in-editor error. Biome remains the
// primary TS linter/formatter — the TS blocks here stay deliberately thin.
import tsParser from '@typescript-eslint/parser';
import svelte from 'eslint-plugin-svelte';
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
    {
        files: ['**/*.ts', '**/*.svelte.ts'],
        languageOptions: {
            parser: tsParser,
        },
        rules: rawInvokeBan,
    },
];
