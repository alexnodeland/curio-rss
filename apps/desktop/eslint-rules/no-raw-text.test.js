// @vitest-environment node
/**
 * `curio/no-raw-text` (#155): untranslated words in a Svelte template —
 * text, user-facing attributes, component text props, action arguments —
 * fail lint; text routed through `t()`, punctuation, code (<style>/<script>)
 * and the configured `allow` tokens pass.
 */
import { RuleTester } from 'eslint';
import svelteParser from 'svelte-eslint-parser';
import { describe, it } from 'vitest';
import { noRawText } from './no-raw-text.js';

RuleTester.describe = describe;
RuleTester.it = it;

const tester = new RuleTester({ languageOptions: { parser: svelteParser } });

const filename = 'Component.svelte';
const rawText = (text) => ({ messageId: 'rawText', data: { text } });

tester.run('curio/no-raw-text', noRawText, {
    valid: [
        { filename, code: "<span>{t('reader.empty')}</span>" },
        { filename, code: '<span aria-hidden="true">·</span><span>⋯ / → 42</span>' },
        { filename, code: "<p title={t('a.b')} aria-label={t('c.d', { name })}>{count}</p>" },
        { filename, code: "<button aria-label={label ?? t('fallback')}>{icon}</button>" },
        { filename, code: '<style>.row { display: flex; color: var(--fg); }</style>' },
        { filename, code: "<script>const label = 'Not template text';</script>" },
        // Attribute values that are not read by people (class, style, data-*).
        {
            filename,
            code: "<div class={on ? 'active' : 'idle'} style:color={'red'} data-kind=\"feed\"></div>",
        },
        // Call arguments are keys, not prose.
        { filename, code: "<span>{t(busy ? 'opml.importing' : 'opml.import')}</span>" },
        // Component props and action arguments routed through t().
        {
            filename,
            code: '<Modal title={t(\'settings.title\')} variant="wide" icon="gear" />',
        },
        {
            filename,
            code: "<b use:tooltip={{ text: t('copy'), kbd: '⌘C' }} use:contextMenu={{ items, ariaLabel: name }}></b>",
        },
        { filename, code: "<b use:tooltip={t('copy')} use:rangeFill={0.5}></b>" },
        // Allowed tokens are not language.
        {
            filename,
            code: '<span>r/{sub}</span><span>HTTP {status}</span>',
            options: [{ allow: ['r/', 'HTTP'] }],
        },
    ],
    invalid: [
        { filename, code: '<span>Loading</span>', errors: [rawText('Loading')] },
        {
            filename,
            code: '<kbd>Enter</kbd> to read',
            errors: [rawText('Enter'), rawText('to read')],
        },
        { filename, code: '<span>{size}px</span>', errors: [rawText('px')] },
        {
            filename,
            code: "<span>{busy ? 'Saving' : label}</span>",
            errors: [rawText('Saving')],
        },
        { filename, code: "<span>{title ?? 'Untitled'}</span>", errors: [rawText('Untitled')] },
        {
            filename,
            code: '<span>{`${count} items`}</span>',
            errors: [rawText('items')],
        },
        {
            filename,
            code: '<input placeholder="Search feeds" title="Find" />',
            errors: [rawText('Search feeds'), rawText('Find')],
        },
        {
            filename,
            code: "<button aria-label={open ? 'Close' : 'Open'}></button>",
            errors: [rawText('Close'), rawText('Open')],
        },
        // A component's text props are copy, static or expression.
        {
            filename,
            code: '<IconButton label="Close" /><Modal heading={open ? \'Settings\' : title} />',
            errors: [rawText('Close'), rawText('Settings')],
        },
        // An action's argument, or the text fields of its options object.
        {
            filename,
            code: "<b use:tooltip={'Copy link'}></b><i use:tooltip={{ text: 'Copy', kbd: '⌘C' }}></i>",
            errors: [rawText('Copy link'), rawText('Copy')],
        },
        {
            filename,
            code: "<b use:contextMenu={{ items, 'ariaLabel': 'Feed actions' }}></b>",
            errors: [rawText('Feed actions')],
        },
        // An allow-listed token does not excuse the words around it.
        {
            filename,
            code: '<span>HTTP status</span>',
            options: [{ allow: ['HTTP'] }],
            errors: [rawText('status')],
        },
    ],
});
