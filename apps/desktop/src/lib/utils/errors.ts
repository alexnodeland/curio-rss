/**
 * Error presentation — the one place the three-tier `CommandError` contract
 * turns into copy. `internal` failures get the generic line so raw internals
 * never leak into the UI (the host logs them, with the command's name). A
 * `user`-tier failure is shown in the active locale: its `code` picks the
 * copy and its `params` fill the copy's `{placeholders}` (a host, an HTTP
 * status, a folder). `message` is the engine's English description — the
 * fallback, and the detail the broad codes keep in English (see `BROAD`).
 */
import type { CommandError, ErrorCode } from '$lib/bindings';
import { type MessageKey, localeStore, t } from '$lib/i18n';

/**
 * The localized copy for every code. Typed as a total record, so a code
 * added on the Rust side without copy fails typecheck here.
 */
export const ERROR_COPY: Record<ErrorCode, MessageKey> = {
    NOT_FOUND: 'error.notFound',
    INVALID_INPUT: 'error.invalidInput',
    INVALID_URL: 'error.invalidUrl',
    PATH_TOKEN_EXPIRED: 'error.pathTokenExpired',
    UNKNOWN_DESTINATION: 'error.unknownDestination',
    DESTINATION_MISSING: 'error.destinationMissing',
    PATHS_TAKEN: 'error.pathsTaken',
    SCHEMA_TOO_NEW: 'error.schemaTooNew',
    NETWORK: 'error.network',
    NETWORK_TIMEOUT: 'error.networkTimeout',
    HOST_NOT_FOUND: 'error.hostNotFound',
    CONNECTION_FAILED: 'error.connectionFailed',
    TLS: 'error.tls',
    HTTP_STATUS: 'error.httpStatus',
    BLOCKED_ADDRESS: 'error.blockedAddress',
    TOO_LARGE: 'error.tooLarge',
    RATE_LIMITED: 'error.rateLimited',
    PARSE: 'error.parse',
    NO_FEED: 'addFeed.noneFound',
    ALREADY_SUBSCRIBED: 'error.alreadySubscribed',
    STORAGE: 'error.storage',
    DUPLICATE: 'error.duplicate',
    DISK_FULL: 'error.diskFull',
    READ_ONLY: 'error.readOnly',
    PROFILE_BUSY: 'app.error.profileBusy',
    IO: 'error.io',
    INTERNAL: 'app.error.internal',
};

/**
 * The broad codes: each covers many distinct engine failures ("invalid
 * destination name: …", "export path has no file name"), so its copy can
 * only be generic. In English the engine's own, more specific message is
 * shown instead; every other locale gets the generic localized line.
 */
const BROAD: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
    'NOT_FOUND',
    'INVALID_INPUT',
    'NETWORK',
    'PARSE',
    'STORAGE',
    'IO',
]);

/** A `{placeholder}` the params did not fill. */
const UNFILLED = /\{\w+\}/;

/** The display string for a command failure, in the active locale. */
export function commandErrorMessage(error: CommandError): string {
    if (error.kind === 'internal') return t('app.error.internal');
    const english = localeStore.active === 'en';
    if (BROAD.has(error.code) && english) return error.message;
    const text = t(ERROR_COPY[error.code], error.params);
    if (!UNFILLED.test(text)) return text;
    // The host promised a param it did not send: never show a raw
    // `{host}` — English has the engine's message, the rest the generic line.
    return english ? error.message : t('app.error.internal');
}

/**
 * The codes a failed fetch carries — the address could not be reached or
 * read, whichever way it failed (the finer codes split the old `NETWORK`).
 */
const UNREACHABLE: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
    'NETWORK',
    'NETWORK_TIMEOUT',
    'HOST_NOT_FOUND',
    'CONNECTION_FAILED',
    'TLS',
    'HTTP_STATUS',
    'BLOCKED_ADDRESS',
    'TOO_LARGE',
    'RATE_LIMITED',
]);

/** Whether `error` is a failed fetch of the address (any network code). */
export function isUnreachable(error: CommandError): boolean {
    return UNREACHABLE.has(error.code);
}
