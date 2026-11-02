<script lang="ts">
/**
 * Settings → Data: the article-retention policy (#137) — an age limit, a
 * per-feed limit and whether unread articles are spared, the same policy
 * `curio retention` edits. Each change is stored at once; the maintenance
 * pass applies it shortly after launch and about daily (or right away
 * from the doctor's "Run maintenance"). Starred, read-later, tagged and
 * exported articles are never deleted, whatever the limits.
 *
 * The controls stay enabled while a change is being stored — disabling
 * one would drop keyboard focus to the page after every arrow press on a
 * select — so the saves are serialized instead: a change made while one
 * is in flight is stored right after it, as the latest policy. A load that
 * fails (a stored policy that no longer parses included) shows the error
 * with a Retry rather than an empty section.
 */
import ErrorState from '$components/common/ErrorState.svelte';
import { type RetentionPolicyDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { toastStore } from '$lib/state/toasts.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import {
    RETENTION_AGE_DAYS,
    RETENTION_PER_FEED,
    limitChoices,
    parseLimit,
} from '$lib/utils/maintenance';

/** What the controls show: the stored policy plus any change not yet stored. */
let policy = $state<RetentionPolicyDto | null>(null);
/** Why the policy could not be loaded, if it could not. */
let loadError = $state<string | null>(null);
let loading = $state(false);

/** The policy as last stored — what the controls snap back to on a failed save. */
let stored: RetentionPolicyDto | null = null;
/** A save is in flight. */
let saving = false;
/** A change arrived while that save was in flight; store it next. */
let queued = false;

async function load(): Promise<void> {
    loading = true;
    try {
        const result = await commands.retentionPolicy();
        if (result.status === 'error') {
            loadError = commandErrorMessage(result.error);
            return;
        }
        loadError = null;
        stored = result.data;
        policy = result.data;
    } catch {
        loadError = t('app.error.internal');
    } finally {
        loading = false;
    }
}
void load();

const ageChoices = $derived(limitChoices(RETENTION_AGE_DAYS, policy?.max_age_days ?? null));
const perFeedChoices = $derived(limitChoices(RETENTION_PER_FEED, policy?.max_per_feed ?? null));

/**
 * Applies `change` to the controls and stores the policy — at once, or
 * right after the save already in flight (one more save then covers every
 * change made meanwhile).
 */
async function update(change: Partial<RetentionPolicyDto>): Promise<void> {
    if (policy === null) {
        return;
    }
    policy = { ...policy, ...change };
    if (saving) {
        queued = true;
        return;
    }
    saving = true;
    try {
        do {
            queued = false;
            await save(policy);
        } while (queued && policy !== null);
    } finally {
        saving = false;
    }
}

/**
 * Stores `next`. On success the controls take the stored form (a `0` limit
 * reads back as none) unless another change is waiting; on failure they
 * snap back to the last stored policy and any waiting change is dropped.
 */
async function save(next: RetentionPolicyDto): Promise<void> {
    try {
        const result = await commands.setRetentionPolicy(next);
        if (result.status === 'error') {
            snapBack();
            toastCommandError(result.error);
            return;
        }
        stored = result.data;
        if (!queued) {
            policy = result.data;
        }
    } catch {
        snapBack();
        toastStore.showToast(t('app.error.internal'), 'error');
    }
}

function snapBack(): void {
    queued = false;
    policy = stored === null ? null : { ...stored };
}
</script>

<section class="retention" aria-labelledby="retention-title">
    <h4 id="retention-title">{t('settings.retention.title')}</h4>
    <p class="hint">{t('settings.retention.hint')}</p>

    {#if policy !== null}
        <label class="field">
            <span class="field-label">{t('settings.retention.maxAge')}</span>
            <select
                class="field-select"
                value={String(policy.max_age_days ?? '')}
                onchange={(event) =>
                    void update({ max_age_days: parseLimit(event.currentTarget.value) })}
            >
                <option value="">{t('settings.retention.maxAge.off')}</option>
                {#each ageChoices as days (days)}
                    <option value={String(days)}>{t('settings.retention.days', { count: days })}</option>
                {/each}
            </select>
        </label>

        <label class="field">
            <span class="field-label">{t('settings.retention.maxPerFeed')}</span>
            <select
                class="field-select"
                value={String(policy.max_per_feed ?? '')}
                onchange={(event) =>
                    void update({ max_per_feed: parseLimit(event.currentTarget.value) })}
            >
                <option value="">{t('settings.retention.maxPerFeed.off')}</option>
                {#each perFeedChoices as count (count)}
                    <option value={String(count)}>{t('settings.retention.newest', { count })}</option>
                {/each}
            </select>
        </label>

        <label class="toggle">
            <input
                type="checkbox"
                checked={policy.keep_unread}
                onchange={(event) => void update({ keep_unread: event.currentTarget.checked })}
            />
            <span class="toggle-text">
                <span class="toggle-label">{t('settings.retention.keepUnread')}</span>
                <span class="toggle-hint">{t('settings.retention.keepUnread.hint')}</span>
            </span>
        </label>

        <p class="hint">{t('settings.retention.schedule')}</p>
    {:else if loadError !== null}
        <ErrorState message={loadError} busy={loading} onretry={() => void load()} />
    {/if}
</section>

<style>
    .retention {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        padding-top: var(--space-4);
        margin-top: var(--space-2);
        border-top: 1px solid var(--hairline);
    }

    h4 {
        font-size: 0.6875rem;
        font-weight: var(--weight-heading);
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .hint {
        font-size: var(--text-sm);
        color: var(--fg-subtle);
        line-height: 1.45;
    }

    /* The field/toggle styles are scoped per component, so this panel carries
       its own copy of the shared Settings styles (as UpdatesPanel does). */
    .field {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
    }

    .field-label {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .field-select {
        padding: var(--space-1) calc(var(--space-3) + 1em) var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background-color: var(--bg-inset, transparent);
        color: var(--fg);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
    }

    .toggle {
        display: flex;
        align-items: flex-start;
        gap: var(--space-3);
        cursor: pointer;
    }

    .toggle input {
        margin-top: 2px;
        width: 16px;
        height: 16px;
        flex: 0 0 auto;
        accent-color: var(--accent);
    }

    .toggle-text {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .toggle-label {
        font-size: var(--text-md);
        color: var(--fg);
    }

    .toggle-hint {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }
</style>
