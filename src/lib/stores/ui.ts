// UI state management

import { writable, derived } from 'svelte/store';
import type { ViewMode } from '$lib/types';

// ============================================================================
// View State
// ============================================================================

export type ViewType = 'all' | 'unread' | 'starred' | 'read_later' | 'feed' | 'folder';

export interface ViewState {
    type: ViewType;
    viewMode: ViewMode;
}

export const viewState = writable<ViewState>({
    type: 'all',
    viewMode: 'article',
});

// ============================================================================
// Sidebar State
// ============================================================================

export const sidebarCollapsed = writable<boolean>(false);
export const sidebarWidth = writable<number>(280);

export function toggleSidebar(): void {
    sidebarCollapsed.update((v) => !v);
}

// ============================================================================
// Modal State
// ============================================================================

export type ModalType = 'add_feed' | 'settings' | 'keyboard_shortcuts' | 'theme_editor' | null;

export const activeModal = writable<ModalType>(null);

export function openModal(modal: ModalType): void {
    activeModal.set(modal);
}

export function closeModal(): void {
    activeModal.set(null);
}

// ============================================================================
// Reader State
// ============================================================================

export const readerFontSize = writable<number>(16);
export const readerLineHeight = writable<number>(1.6);
export const readerMaxWidth = writable<number>(720);

export function increaseFontSize(): void {
    readerFontSize.update((size) => Math.min(size + 2, 32));
}

export function decreaseFontSize(): void {
    readerFontSize.update((size) => Math.max(size - 2, 12));
}

// ============================================================================
// Theme State
// ============================================================================

export type ThemeId =
    | 'light'
    | 'dark'
    | 'nord'
    | 'catppuccin'
    | 'dracula'
    | 'gruvbox'
    | 'tokyo-night'
    | 'solarized-dark'
    | 'solarized-light';

export interface ThemeInfo {
    id: ThemeId;
    name: string;
    isDark: boolean;
}

export const THEMES: ThemeInfo[] = [
    { id: 'light', name: 'Light', isDark: false },
    { id: 'dark', name: 'Dark', isDark: true },
    { id: 'nord', name: 'Nord', isDark: true },
    { id: 'catppuccin', name: 'Catppuccin Mocha', isDark: true },
    { id: 'dracula', name: 'Dracula', isDark: true },
    { id: 'gruvbox', name: 'Gruvbox Dark', isDark: true },
    { id: 'tokyo-night', name: 'Tokyo Night', isDark: true },
    { id: 'solarized-dark', name: 'Solarized Dark', isDark: true },
    { id: 'solarized-light', name: 'Solarized Light', isDark: false },
];

const THEME_STORAGE_KEY = 'curio-theme';

function getStoredTheme(): ThemeId {
    if (typeof localStorage === 'undefined') return 'dark';
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    if (stored && THEMES.some((t) => t.id === stored)) {
        return stored as ThemeId;
    }
    // Check system preference
    if (typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: light)').matches) {
        return 'light';
    }
    return 'dark';
}

export const currentTheme = writable<ThemeId>(getStoredTheme());

export function setTheme(theme: ThemeId): void {
    currentTheme.set(theme);
    // Apply theme class to document
    if (typeof document !== 'undefined') {
        document.documentElement.setAttribute('data-theme', theme);
    }
    // Persist to localStorage
    if (typeof localStorage !== 'undefined') {
        localStorage.setItem(THEME_STORAGE_KEY, theme);
    }
}

export function initializeTheme(): void {
    const theme = getStoredTheme();
    setTheme(theme);
}

export function getThemeInfo(id: ThemeId): ThemeInfo | undefined {
    return THEMES.find((t) => t.id === id);
}

// ============================================================================
// Search State
// ============================================================================

export const searchQuery = writable<string>('');
export const searchResults = writable<string[]>([]);
export const isSearching = writable<boolean>(false);

export function clearSearch(): void {
    searchQuery.set('');
    searchResults.set([]);
}

// ============================================================================
// Keyboard Navigation
// ============================================================================

export const focusedElement = writable<'sidebar' | 'list' | 'reader'>('list');

export function setFocus(element: 'sidebar' | 'list' | 'reader'): void {
    focusedElement.set(element);
}

// ============================================================================
// Toast Notifications
// ============================================================================

export interface Toast {
    id: string;
    message: string;
    type: 'info' | 'success' | 'warning' | 'error';
    duration?: number;
}

export const toasts = writable<Toast[]>([]);

let toastId = 0;

export function showToast(
    message: string,
    type: Toast['type'] = 'info',
    duration: number = 3000,
): void {
    const id = `toast-${++toastId}`;
    const toast: Toast = { id, message, type, duration };

    toasts.update((t) => [...t, toast]);

    if (duration > 0) {
        setTimeout(() => {
            dismissToast(id);
        }, duration);
    }
}

export function dismissToast(id: string): void {
    toasts.update((t) => t.filter((toast) => toast.id !== id));
}
