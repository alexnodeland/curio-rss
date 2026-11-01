# Newsletters as feeds

Curio reads feeds, not mailboxes. Most email newsletters are easy to turn
into a feed, though, and once a newsletter is a feed it gets everything
other subscriptions get: read state, full-text search, folders, and
saving to your notes.

## Newsletters that already publish a feed

Many newsletter platforms put every issue on the web, so you can subscribe
without an inbox. Paste the newsletter's web address into **Add a feed**
(desktop) or `curio feed add` (CLI), and discovery finds the feed:

| Platform | Paste | Curio subscribes to |
|---|---|---|
| Substack | `https://<name>.substack.com` (any page of it) | `https://<name>.substack.com/feed` |
| Buttondown, Ghost, WordPress, … | the newsletter's archive or home page | the feed the page declares, or one found at a common path such as `/feed` or `/rss` |

Discovery fetches every candidate and checks that it parses before it
offers it. If the page has no feed, Curio says so instead of subscribing
to a page that would fail on every refresh.

## Newsletters that only arrive by email

For a newsletter with no public archive, use an email-to-feed bridge.
[Kill the Newsletter](https://kill-the-newsletter.com/) is a free one:

1. Open Kill the Newsletter and create an inbox. Give it the newsletter's
   name so you can tell your inboxes apart. You get an email address
   (`<id>@kill-the-newsletter.com`) and a feed URL
   (`https://kill-the-newsletter.com/feeds/<id>.xml`).
2. Sign up for the newsletter with that email address. If the newsletter
   sends a confirmation email, it shows up as the first item in the feed;
   open it and confirm.
3. Add the feed to Curio.
   - **Desktop:** in **Add a feed**, choose **Newsletter** under *Popular
     sources*. It explains these steps and links to Kill the Newsletter.
     Paste either the inbox's **email address** or its **feed URL**. Curio
     recognizes both, shows the feed it will subscribe to, and suggests
     the folder `Newsletters`.
   - **CLI:** `curio feed add https://kill-the-newsletter.com/feeds/<id>.xml --tags Newsletters`

Use one inbox per newsletter, so each one is its own feed with its own
read state and folder.

### Things to know

- **Privacy.** The bridge receives your newsletters, so it can read them.
  Curio never contacts it except to fetch the feed, the same as any other
  subscription. If that is a concern, Kill the Newsletter is open source
  and can be self-hosted. A self-hosted inbox's feed URL works like any
  feed URL (Curio's address shortcut only recognizes the public
  `kill-the-newsletter.com` instance).
- **Unsubscribing.** Removing the feed in Curio does not stop the emails
  arriving at the bridge. Unsubscribe from the newsletter itself (its
  unsubscribe link is in every issue), then remove the inbox from the
  bridge.
- **New feeds start empty.** An inbox's feed has nothing in it until the
  first email arrives, but Curio still subscribes, because an empty feed
  is a valid feed. Issues appear on the next refresh after they land.

## Release notes are feeds too

GitHub publishes an Atom feed of every repository's releases. In **Add a
feed**, choose **GitHub releases** and finish the repository address
(`https://github.com/<owner>/<repo>`), or paste a repository URL anywhere.
Curio subscribes to `https://github.com/<owner>/<repo>/releases.atom`
(`/tags.atom` for a `/tags` URL) and suggests the folder `Dev/GitHub`.
`curio feed add https://github.com/<owner>/<repo>` does the same from the
CLI.
