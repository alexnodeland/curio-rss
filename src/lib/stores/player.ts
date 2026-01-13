// Podcast player state management

import { writable, derived, get } from 'svelte/store';
import type { Article } from '$lib/types';

// ============================================================================
// Player State
// ============================================================================

export interface PlayerState {
    currentEpisode: Article | null;
    isPlaying: boolean;
    currentTime: number;
    duration: number;
    volume: number;
    playbackRate: number;
    isMuted: boolean;
}

const defaultState: PlayerState = {
    currentEpisode: null,
    isPlaying: false,
    currentTime: 0,
    duration: 0,
    volume: 1,
    playbackRate: 1,
    isMuted: false,
};

export const playerState = writable<PlayerState>(defaultState);

// ============================================================================
// Player Queue
// ============================================================================

export interface QueueItem {
    article: Article;
    position: number;
}

export const playerQueue = writable<QueueItem[]>([]);

// ============================================================================
// Derived Stores
// ============================================================================

export const currentEpisode = derived(playerState, ($state) => $state.currentEpisode);

export const isPlaying = derived(playerState, ($state) => $state.isPlaying);

export const currentTime = derived(playerState, ($state) => $state.currentTime);

export const duration = derived(playerState, ($state) => $state.duration);

export const progress = derived(playerState, ($state) => {
    if ($state.duration === 0) return 0;
    return ($state.currentTime / $state.duration) * 100;
});

export const remainingTime = derived(playerState, ($state) => {
    return Math.max(0, $state.duration - $state.currentTime);
});

export const hasQueue = derived(playerQueue, ($queue) => $queue.length > 0);

export const queueLength = derived(playerQueue, ($queue) => $queue.length);

// ============================================================================
// Player Controls
// ============================================================================

export function playEpisode(article: Article): void {
    playerState.update((state) => ({
        ...state,
        currentEpisode: article,
        isPlaying: true,
        currentTime: article.podcast_progress || 0,
        duration: article.podcast_duration || 0,
    }));
}

export function play(): void {
    playerState.update((state) => ({
        ...state,
        isPlaying: true,
    }));
}

export function pause(): void {
    playerState.update((state) => ({
        ...state,
        isPlaying: false,
    }));
}

export function togglePlayPause(): void {
    playerState.update((state) => ({
        ...state,
        isPlaying: !state.isPlaying,
    }));
}

export function stop(): void {
    playerState.set(defaultState);
}

export function seek(time: number): void {
    playerState.update((state) => ({
        ...state,
        currentTime: Math.max(0, Math.min(time, state.duration)),
    }));
}

export function seekRelative(delta: number): void {
    playerState.update((state) => ({
        ...state,
        currentTime: Math.max(0, Math.min(state.currentTime + delta, state.duration)),
    }));
}

export function skipForward(seconds: number = 30): void {
    seekRelative(seconds);
}

export function skipBack(seconds: number = 15): void {
    seekRelative(-seconds);
}

export function setVolume(volume: number): void {
    playerState.update((state) => ({
        ...state,
        volume: Math.max(0, Math.min(volume, 1)),
        isMuted: false,
    }));
}

export function toggleMute(): void {
    playerState.update((state) => ({
        ...state,
        isMuted: !state.isMuted,
    }));
}

export function setPlaybackRate(rate: number): void {
    // Common rates: 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2
    const clampedRate = Math.max(0.5, Math.min(rate, 3));
    playerState.update((state) => ({
        ...state,
        playbackRate: clampedRate,
    }));
}

export function cyclePlaybackRate(): void {
    const rates = [1, 1.25, 1.5, 1.75, 2, 0.75];
    const state = get(playerState);
    const currentIndex = rates.indexOf(state.playbackRate);
    const nextIndex = currentIndex === -1 ? 0 : (currentIndex + 1) % rates.length;
    setPlaybackRate(rates[nextIndex]);
}

export function updateProgress(currentTime: number, duration: number): void {
    playerState.update((state) => ({
        ...state,
        currentTime,
        duration,
    }));
}

// ============================================================================
// Queue Management
// ============================================================================

export function addToQueue(article: Article): void {
    const state = get(playerQueue);
    const position = state.length;
    playerQueue.update((queue) => [...queue, { article, position }]);
}

export function removeFromQueue(articleId: string): void {
    playerQueue.update((queue) =>
        queue
            .filter((item) => item.article.id !== articleId)
            .map((item, index) => ({ ...item, position: index }))
    );
}

export function clearQueue(): void {
    playerQueue.set([]);
}

export function playNext(): void {
    const state = get(playerState);
    const queue = get(playerQueue);

    if (queue.length === 0) {
        stop();
        return;
    }

    const nextItem = queue[0];
    playerQueue.update((q) =>
        q.slice(1).map((item, index) => ({ ...item, position: index }))
    );

    playEpisode(nextItem.article);
}

export function moveInQueue(fromIndex: number, toIndex: number): void {
    playerQueue.update((queue) => {
        const newQueue = [...queue];
        const [moved] = newQueue.splice(fromIndex, 1);
        newQueue.splice(toIndex, 0, moved);
        return newQueue.map((item, index) => ({ ...item, position: index }));
    });
}

export function shuffleQueue(): void {
    playerQueue.update((queue) => {
        const shuffled = [...queue];
        for (let i = shuffled.length - 1; i > 0; i--) {
            const j = Math.floor(Math.random() * (i + 1));
            [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
        }
        return shuffled.map((item, index) => ({ ...item, position: index }));
    });
}

// ============================================================================
// Persistence Helpers
// ============================================================================

const PLAYER_STATE_KEY = 'curio-player-state';
const QUEUE_KEY = 'curio-player-queue';

export function savePlayerState(): void {
    if (typeof localStorage === 'undefined') return;

    const state = get(playerState);
    if (state.currentEpisode) {
        localStorage.setItem(
            PLAYER_STATE_KEY,
            JSON.stringify({
                episodeId: state.currentEpisode.id,
                currentTime: state.currentTime,
                volume: state.volume,
                playbackRate: state.playbackRate,
            })
        );
    }
}

export function saveQueue(): void {
    if (typeof localStorage === 'undefined') return;

    const queue = get(playerQueue);
    const queueIds = queue.map((item) => item.article.id);
    localStorage.setItem(QUEUE_KEY, JSON.stringify(queueIds));
}

export function loadSavedState(): { episodeId?: string; currentTime?: number; volume?: number; playbackRate?: number } | null {
    if (typeof localStorage === 'undefined') return null;

    const saved = localStorage.getItem(PLAYER_STATE_KEY);
    if (saved) {
        try {
            return JSON.parse(saved);
        } catch {
            return null;
        }
    }
    return null;
}

export function loadSavedQueue(): string[] {
    if (typeof localStorage === 'undefined') return [];

    const saved = localStorage.getItem(QUEUE_KEY);
    if (saved) {
        try {
            return JSON.parse(saved);
        } catch {
            return [];
        }
    }
    return [];
}

// ============================================================================
// Time Formatting Helpers
// ============================================================================

export function formatPlayerTime(seconds: number): string {
    if (!Number.isFinite(seconds)) return '0:00';

    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);

    if (h > 0) {
        return `${h}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
    }
    return `${m}:${s.toString().padStart(2, '0')}`;
}

export function formatRemainingTime(seconds: number): string {
    return `-${formatPlayerTime(seconds)}`;
}
