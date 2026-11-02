// `curio/no-raw-text` — every word a user reads in a Svelte template comes
// from the i18n catalogs through `t()` (#155). The rule reports:
//
//   - template text nodes containing letters (`<span>Loading</span>`),
//   - string literals rendered as text (`{busy ? 'Saving' : label}`,
//     `{name ?? 'Untitled'}`, template-literal chunks),
//   - the same in user-facing attributes (`aria-label`, `title`,
//     `placeholder`, `alt`, …), whether static or an expression,
//   - the same in a component's text props (`<Modal title="…">`,
//     `<EmptyState message={…}>`),
//   - the same in an action's argument (`use:tooltip={'Copy'}`), or in the
//     text fields of an options object (`use:tooltip={{ text: 'Copy' }}`,
//     `use:contextMenu={{ ariaLabel: 'Feed' }}`).
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

/**
 * Component props that carry copy. Every attribute above counts on a
 * component too; these are the names the components under src/lib give
 * their text props (`<Modal title>`, `<ErrorState message>`, …).
 */
const COMPONENT_TEXT_PROPS = new Set([
    ...USER_FACING_ATTRIBUTES,
    'ariaLabel',
    'description',
    'heading',
    'hint',
    'label',
    'message',
    'text',
    'tooltip',
]);

/** Fields of an action's options object that carry copy (`{ text, kbd }`). */
const ACTION_TEXT_FIELDS = new Set(['ariaLabel', 'label', 'text', 'title']);

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

/** Whether an attribute's value is read by people: a user-facing attribute,
 *  or on a component, one of its text props. */
function carriesCopy(attribute) {
    const element = attribute.parent.parent;
    const names = element.kind === 'component' ? COMPONENT_TEXT_PROPS : USER_FACING_ATTRIBUTES;
    return names.has(attribute.key.name);
}

/** The strings an attribute value renders: its static parts, and the string
 *  literals of its `{…}` parts. */
function attributeStrings(attribute) {
    return attribute.value.flatMap((part) => {
        if (part.type === 'SvelteLiteral') {
            return [{ node: part, text: part.value }];
        }
        return part.type === 'SvelteMustacheTag' ? renderedStrings(part.expression) : [];
    });
}

/** The string literals an action argument renders: the argument itself, or
 *  the text fields of an options object. */
function actionStrings(expression) {
    if (expression?.type !== 'ObjectExpression') {
        return renderedStrings(expression);
    }
    return expression.properties.flatMap((property) => {
        const key = property.type === 'Property' && !property.computed ? property.key : null;
        const name = key?.type === 'Identifier' ? key.name : key?.value;
        return ACTION_TEXT_FIELDS.has(name) ? renderedStrings(property.value) : [];
    });
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

        function checkStrings(literals) {
            for (const literal of literals) {
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
                // `SvelteAttribute` / `SvelteDirective` below (or are not
                // user-facing at all).
                if (node.kind === 'text' && !/Attribute|Directive/.test(node.parent.type)) {
                    checkStrings(renderedStrings(node.expression));
                }
            },
            SvelteAttribute(node) {
                if (carriesCopy(node)) {
                    checkStrings(attributeStrings(node));
                }
            },
            SvelteDirective(node) {
                // `use:action={…}` — other directives (class:, style:, bind:,
                // on:, transition:) take values nobody reads as copy.
                if (node.kind === 'Action') {
                    checkStrings(actionStrings(node.expression));
                }
            },
        };
    },
};
