/**
 * English message catalog — the only locale in v1. Every user-visible string
 * routes through `t()` (retrofit-proofing: translations post-1.0 swap the
 * catalog, not the call sites). Placeholders use `{name}` syntax.
 */
export const en = {
    'app.title': 'Curio',
    'app.tagline': 'Your reading, in your notes',
    'app.loading': 'Loading…',
    'app.error.internal': 'Something went wrong. Retrying may help.',

    'shell.feeds.loading': 'Loading subscriptions…',
    'shell.feeds.empty': 'No subscriptions yet',
    'shell.feeds.count': '{count} feeds',
    'shell.unread.count': '{count} unread',

    'view.all': 'All articles',
    'view.starred': 'Starred',
    'view.readLater': 'Read later',
    'view.feeds': 'Feeds',

    'toast.dismiss': 'Dismiss',

    'shortcut.category.navigation': 'Navigation',
    'shortcut.category.actions': 'Actions',
    'shortcut.category.views': 'Views',
    'shortcut.category.app': 'Application',

    'shortcut.nextArticle': 'Next article',
    'shortcut.previousArticle': 'Previous article',
    'shortcut.openArticle': 'Open article in browser',
    'shortcut.toggleStar': 'Star / unstar article',
    'shortcut.toggleReadLater': 'Add to / remove from read later',
    'shortcut.toggleRead': 'Mark read / unread',
    'shortcut.promote': 'Promote article to a destination',
    'shortcut.refreshFeed': 'Refresh the selected feed',
    'shortcut.refreshAll': 'Refresh all feeds',
    'shortcut.search': 'Search articles',
    'shortcut.viewAll': 'Go to all articles',
    'shortcut.viewStarred': 'Go to starred',
    'shortcut.viewReadLater': 'Go to read later',
    'shortcut.viewFeeds': 'Go to the feed list',
    'shortcut.help': 'Keyboard shortcut help',
} as const;
