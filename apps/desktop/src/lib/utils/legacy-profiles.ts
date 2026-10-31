/**
 * Profiles from an earlier release. Before 0.5 the app and the command line
 * kept their libraries in different places. At startup the app moves a lone
 * old library to the location they now share, or leaves it where it is and
 * says why. When an empty new library would strand the old one, the app
 * opens the old one where it is instead. This module turns those outcomes
 * into the startup notice and the doctor panel's explanation.
 */
import { type LegacyProfileDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { type ToastTone, toastStore } from '$lib/state/toasts.svelte';

/** How long the "moved" notice stays; warnings stay until dismissed. */
export const MOVED_NOTICE_MS = 8000;

/** This session's outcomes; empty when there were none or the call failed. */
export async function loadLegacyProfiles(): Promise<LegacyProfileDto[]> {
    try {
        const result = await commands.legacyProfiles();
        return result.status === 'ok' ? result.data : [];
    } catch {
        return [];
    }
}

/** Where the full explanation lives: Settings › Advanced (the doctor panel). */
function detailsLocation(): string {
    return `${t('settings.title')} › ${t('settings.section.advanced')}`;
}

/** The short startup notice for one outcome. */
export function legacyProfileNotice(dto: LegacyProfileDto): { message: string; tone: ToastTone } {
    if (dto.status === 'moved') {
        return { message: t('legacy.notice.moved', { path: dto.default_dir }), tone: 'info' };
    }
    const key = dto.opened_in_place ? 'legacy.notice.openedInPlace' : 'legacy.notice.left';
    return { message: t(key, { path: dto.path, where: detailsLocation() }), tone: 'warning' };
}

/** The doctor panel's sentence: what happened and what to do about it. */
export function legacyProfileDetail(dto: LegacyProfileDto): string {
    const params = { path: dto.path, default: dto.default_dir, detail: dto.detail ?? '' };
    let text: string;
    switch (dto.status) {
        case 'moved':
            return t('legacy.detail.moved', params);
        case 'default_in_use':
            text = t('legacy.reason.defaultInUse', params);
            break;
        case 'ambiguous':
            text = t('legacy.reason.ambiguous', params);
            break;
        case 'in_use':
            text = t('legacy.reason.inUse', params);
            break;
        case 'default_not_empty':
            text = t('legacy.reason.defaultNotEmpty', params);
            break;
        case 'unreadable':
            text = t('legacy.reason.unreadable', params);
            break;
        case 'move_failed':
            text = t('legacy.reason.moveFailed', params);
            break;
    }
    return dto.opened_in_place ? `${text} ${t('legacy.detail.openedInPlace')}` : text;
}

/** Shows the startup notices, one per outcome. */
export async function announceLegacyProfiles(): Promise<void> {
    for (const dto of await loadLegacyProfiles()) {
        const { message, tone } = legacyProfileNotice(dto);
        toastStore.showToast(message, tone, tone === 'info' ? MOVED_NOTICE_MS : 0);
    }
}
