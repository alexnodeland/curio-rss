/**
 * The reader's session bookkeeping (#153): real dwell time for the
 * `article.opened` contract event, and the saved reading position.
 *
 * A *session* is one stretch of an article sitting in the reader pane. Its
 * dwell counts only the time the window is in the foreground (visible and
 * focused), so an article left open overnight, or behind another app, does
 * not read as hours of reading. The session ends when the selection moves
 * on, the reader closes, or the page goes to the background or away; only
 * then is the dwell known, so that is when `article.opened` is recorded —
 * with a real `dwell_ms`. A pass shorter than {@link DWELL_MIN_MS} (j/k
 * scanning past an article) was not an open-for-reading and records nothing.
 *
 * The position rides alongside: the reader reports it as it scrolls, and it
 * is saved once the scroll settles ({@link PROGRESS_SAVE_MS}) and again as
 * the session ends, so a reopen resumes where reading stopped.
 *
 * Both writes are fire-and-forget: a failure is logged by the Rust command
 * layer, and a toast for a background save the user never asked for would
 * only be noise.
 */
import { commands } from '$lib/bindings';

/** The shortest stay that counts as opening an article for reading. */
export const DWELL_MIN_MS = 1500;

/** How long the scroll must settle before the position is saved. */
export const PROGRESS_SAVE_MS = 800;

/**
 * Sends a background write without waiting on it. A contract error comes
 * back as a value (already logged Rust-side); a thrown one — no Tauri
 * runtime — is dropped here rather than surfacing as an unhandled rejection.
 */
function fireAndForget(write: Promise<unknown>): void {
    write.catch(() => undefined);
}

class ReadingSession {
    #articleId: number | null = null;
    /** Foreground time banked from finished stretches. */
    #dwellMs = 0;
    /** When the running foreground stretch began (`Date.now()`), if one is. */
    #since: number | null = null;
    /** The newest position not yet saved. */
    #pending: number | null = null;
    /** The position this session last saved (an unmoved one is not re-sent). */
    #saved: number | null = null;
    #saveTimer: ReturnType<typeof setTimeout> | null = null;

    /** The article this session is on, if any. */
    get articleId(): number | null {
        return this.#articleId;
    }

    /**
     * Starts a session on `articleId`, ending any other one first (a repeat
     * call for the running article is a no-op). `active` says whether the
     * window is in the foreground right now.
     */
    begin(articleId: number, active: boolean): void {
        if (this.#articleId === articleId) {
            return;
        }
        this.end();
        this.#articleId = articleId;
        this.#dwellMs = 0;
        this.#since = active ? Date.now() : null;
        this.#saved = null;
    }

    /** The window came to (`true`) or left (`false`) the foreground. */
    setActive(active: boolean): void {
        if (this.#articleId === null) {
            return;
        }
        if (active && this.#since === null) {
            this.#since = Date.now();
        } else if (!active && this.#since !== null) {
            this.#dwellMs += Date.now() - this.#since;
            this.#since = null;
        }
    }

    /** Foreground milliseconds so far in this session. */
    dwellMs(): number {
        const running = this.#since === null ? 0 : Date.now() - this.#since;
        return Math.round(this.#dwellMs + running);
    }

    /**
     * Ends the session: saves the position, then records `article.opened`
     * with the dwell — if the article stayed long enough to count.
     */
    end(): void {
        const articleId = this.#articleId;
        if (articleId === null) {
            return;
        }
        this.#flushProgress();
        const dwell = this.dwellMs();
        this.#articleId = null;
        this.#dwellMs = 0;
        this.#since = null;
        this.#saved = null;
        if (dwell >= DWELL_MIN_MS) {
            fireAndForget(commands.recordOpened(articleId, dwell));
        }
    }

    /**
     * The session's article is being opened in the browser: that is an open
     * whatever the dwell, so it is recorded now with the time read so far,
     * and the count restarts for any reading that follows. `false` when
     * `articleId` is not the session's — the caller records that open itself.
     */
    recordExternalOpen(articleId: number): boolean {
        if (this.#articleId !== articleId) {
            return false;
        }
        const dwell = this.dwellMs();
        this.#dwellMs = 0;
        this.#since = this.#since === null ? null : Date.now();
        fireAndForget(commands.recordOpened(articleId, dwell));
        return true;
    }

    /** The reader scrolled to `permille`; saved once the scroll settles. */
    trackProgress(articleId: number, permille: number): void {
        if (articleId !== this.#articleId) {
            return;
        }
        this.#pending = permille;
        this.#clearTimer();
        this.#saveTimer = setTimeout(() => this.#flushProgress(), PROGRESS_SAVE_MS);
    }

    #flushProgress(): void {
        this.#clearTimer();
        const articleId = this.#articleId;
        const permille = this.#pending;
        this.#pending = null;
        if (articleId === null || permille === null || permille === this.#saved) {
            return;
        }
        this.#saved = permille;
        fireAndForget(commands.setReadProgress(articleId, permille));
    }

    #clearTimer(): void {
        if (this.#saveTimer !== null) {
            clearTimeout(this.#saveTimer);
            this.#saveTimer = null;
        }
    }
}

/** The one reading session — the reader pane shows one article at a time. */
export const readingSession = new ReadingSession();
