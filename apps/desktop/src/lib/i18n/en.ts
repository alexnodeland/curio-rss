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

    'sidebar.label': 'Subscriptions',
    'sidebar.views': 'Views',
    'sidebar.feeds': 'Feeds',

    'list.label': 'Articles',
    'list.loading': 'Loading articles…',
    'list.empty': 'No articles here',
    'list.row.unread': 'Unread',
    'list.row.starred': 'Starred',

    'reader.empty': 'Select an article to read',
    'reader.missing': 'This article no longer exists',
    'reader.toolbar': 'Article actions',
    'reader.action.markRead': 'Mark read',
    'reader.action.markUnread': 'Mark unread',
    'reader.action.star': 'Star',
    'reader.action.unstar': 'Unstar',
    'reader.action.readLater': 'Read later',
    'reader.action.readLaterRemove': 'Remove from read later',
    'reader.action.archive': 'Archive',
    'reader.action.unarchive': 'Unarchive',
    'reader.action.openInBrowser': 'Open in browser',
    'reader.action.promote': 'Save to notes',
    'reader.action.typography': 'Typography',
    'reader.meta.words': '{count} words',

    'reader.youtube.play': 'Play {title}',
    'reader.youtube.hint': 'Click to load — nothing loads until you do',
    'reader.reddit.openThread': 'Open discussion on Reddit',

    'typography.title': 'Typography',
    'typography.font': 'Font',
    'typography.size': 'Size',
    'typography.lineHeight': 'Line height',
    'typography.measure': 'Width',
    'typography.reset': 'Reset',

    'tags.label': 'Tags',
    'tags.placeholder': 'Add a tag…',
    'tags.add': 'Add tag',
    'tags.remove': 'Remove tag {tag}',
    'tags.empty': 'No tags yet',

    'search.label': 'Search',
    'search.placeholder': 'Search articles',
    'search.clear': 'Clear search',
    'search.loading': 'Searching…',
    'search.empty': 'No matches',
    'search.results': '{count} results',

    'destinations.title': 'Destinations',
    'destinations.open': 'Destinations',
    'destinations.empty': 'No destinations yet — add one to promote articles into your notes.',
    'destinations.loading': 'Loading destinations…',
    'destinations.name': 'Name',
    'destinations.namePlaceholder': 'e.g. notes',
    'destinations.chooseFolder': 'Choose folder…',
    'destinations.add': 'Add destination',
    'destinations.remove': 'Remove {name}',
    'destinations.default': 'Default',
    'destinations.makeDefault': 'Make default',
    'destinations.needNameAndFolder': 'Enter a name and choose a folder first.',

    'toast.promote.saved': 'Saved to {name}',
    'toast.promote.unchanged': 'Already up to date in {name}',

    'help.title': 'Keyboard shortcuts',
    'help.close': 'Close',
    'help.chord.then': 'then',

    'pane.sidebar.resize': 'Resize sidebar',
    'pane.list.resize': 'Resize article list',

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
