// `curio/no-raw-text` — every word a user reads in a Svelte template comes
// from the i18n catalogs through `t()` (#155). The rule reports:
//
//   - template text nodes containing letters (`<span>Loading</span>`),
//   - string literals rendered as text (`{busy ? 'Saving' : label}`,
//     `{name ?? 'Untitled'}`, template-literal chunks),
//   - the same in user-facing attributes (`aria-label`, `title`,
//     `placeholder`, `alt`, …), whether static or an expression.
//
// Arguments of a call are never inspected — `t('some.key')` passes a key,
// not prose. Punctuation, digits and symbols (`·`, `⋯`, `/`) are fine; only
// a token with a letter in it needs a catalog entry. The `allow` option
// lists the tokens that are not language and so read the same in every
// locale (protocol names, brand wordmarks); each entry carries its reason
// where the rule is configured.

/** Attributes whose value is read by people (directly or via assistive tech). */
const USER_FACING_ATTRIBUTES = new Set([
    'alt',
    'aria-description',
    'aria-label',
    'aria-placeholder',
    'aria-roledescription',
    'aria-valuetext',
    'placeholder',
    'title',
]);

/** Elements whose text children are source code. */
const CODE_ELEMENTS = new Set(['SvelteStyleElement', 'SvelteScriptElement']);

const LETTER = /\p{L}/u;

/** The words in `text` that need a translation: tokens with a letter, not allowed. */
function untranslatedWords(text, allowed) {
    return text.split(/\s+/).filter((token) => LETTER.test(token) && !allowed.has(token));
}

/** The string literals an expression renders as text (never call arguments). */
function renderedStrings(expression) {
    if (expression === null || expression === undefined) {
        return [];
    }
    switch (expression.type) {
        case 'Literal':
            return typeof expression.value === 'string'
                ? [{ node: expression, text: expression.value }]
                : [];
        case 'TemplateLiteral':
            return [
                ...expression.quasis.map((quasi) => ({
                    node: quasi,
                    text: quasi.value.cooked ?? '',
                })),
                ...expression.expressions.flatMap(renderedStrings),
            ];
        case 'ConditionalExpression':
            return [
                ...renderedStrings(expression.consequent),
                ...renderedStrings(expression.alternate),
            ];
        case 'LogicalExpression':
            return [...renderedStrings(expression.left), ...renderedStrings(expression.right)];
        default:
            return [];
    }
}

/** @type {import('eslint').Rule.RuleModule} */
export const noRawText = {
    meta: {
        type: 'problem',
        docs: {
            description: 'Disallow untranslated text in Svelte templates — route it through t()',
        },
        schema: [
            {
                type: 'object',
                properties: {
                    allow: { type: 'array', items: { type: 'string' }, uniqueItems: true },
                },
                additionalProperties: false,
            },
        ],
        messages: {
            rawText:
                'Untranslated text "{{text}}" — add a catalog key in all 8 locales and render it with t().',
        },
    },
    create(context) {
        const allowed = new Set(context.options[0]?.allow ?? []);

        function check(node, text) {
            const words = untranslatedWords(text, allowed);
            if (words.length > 0) {
                context.report({ node, messageId: 'rawText', data: { text: words.join(' ') } });
            }
        }

        function checkExpression(expression) {
            for (const literal of renderedStrings(expression)) {
                check(literal.node, literal.text);
            }
        }

        return {
            SvelteText(node) {
                // A component's <style> / <script> body is code, not copy.
                if (CODE_ELEMENTS.has(node.parent.type)) {
                    return;
                }
                check(node, node.value);
            },
            SvelteMustacheTag(node) {
                // Attribute and directive values are routed through
                // `SvelteAttribute` below (or are not user-facing at all).
                if (node.kind === 'text' && !/Attribute|Directive/.test(node.parent.type)) {
                    checkExpression(node.expression);
                }
            },
            SvelteAttribute(node) {
                if (!USER_FACING_ATTRIBUTES.has(node.key.name)) {
                    return;
                }
                for (const part of node.value) {
                    if (part.type === 'SvelteLiteral') {
                        check(part, part.value);
                    } else if (part.type === 'SvelteMustacheTag') {
                        checkExpression(part.expression);
                    }
                }
            },
        };
    },
};
