/**
 * Keeps a range input's `--pct` custom property in sync with its value, so the
 * shared slider style (app.css) can paint the filled portion of the track with
 * a gradient. WebKit's `::-webkit-slider-runnable-track` has no native
 * "progress" pseudo (unlike Firefox's `::-moz-range-progress`), so the fill has
 * to be driven from JS — this is what makes the packaged WKWebView build match
 * dev Chromium. Pass the current value as the action argument so a programmatic
 * change (reset, settings) refreshes the fill, not just user dragging.
 */
export function rangeFill(node: HTMLInputElement, _value?: number) {
    const update = (): void => {
        const min = Number(node.min === '' ? 0 : node.min);
        const max = Number(node.max === '' ? 100 : node.max);
        const val = Number(node.value);
        const pct = max > min ? ((val - min) / (max - min)) * 100 : 0;
        node.style.setProperty('--pct', `${Math.max(0, Math.min(100, pct))}%`);
    };
    update();
    node.addEventListener('input', update);
    return {
        update(): void {
            update();
        },
        destroy(): void {
            node.removeEventListener('input', update);
        },
    };
}
