import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

export default defineConfig({
    plugins: [svelte({ hot: false })],
    test: {
        include: ['tests/unit/**/*.test.ts', 'src/**/*.test.ts'],
        environment: 'jsdom',
        globals: true,
        coverage: {
            provider: 'v8',
            reporter: ['text', 'lcov', 'html'],
            reportsDirectory: 'coverage/frontend',
            exclude: [
                'node_modules/',
                'tests/',
                '**/*.d.ts',
                'src/lib/types/generated/**',
                '.svelte-kit/**',
            ],
            thresholds: {
                lines: 80,
                branches: 75,
                functions: 80,
                statements: 80,
            },
        },
    },
    resolve: {
        alias: {
            $lib: '/src/lib',
            $components: '/src/lib/components',
        },
    },
});
