import type { MessageKey } from './index';

/**
 * Spanish (partial). A locale ships whatever it has translated; `t()` falls
 * back to English for any key absent here, so a partial catalog is a
 * first-class, shippable state — and the proof that string routing works
 * with more than one language behind it. Proper nouns (Curio, the theme
 * names) are intentionally left to English.
 */
export const es: Partial<Record<MessageKey, string>> = {
    'app.tagline': 'Tu lectura, en tus notas',
    'app.loading': 'Cargando…',
    'app.error.internal': 'Algo salió mal. Reintentar puede ayudar.',

    'a11y.skipToContent': 'Saltar al contenido',

    'shell.feeds.loading': 'Cargando suscripciones…',
    'shell.feeds.empty': 'Aún no hay suscripciones',

    'list.label': 'Artículos',
    'list.loading': 'Cargando artículos…',
    'list.empty': 'No hay artículos aquí',

    'reader.empty': 'Selecciona un artículo para leer',

    'modal.close': 'Cerrar',
    'toast.dismiss': 'Descartar',
    'help.title': 'Atajos de teclado',

    'settings.section.reading': 'Lectura',
    'settings.markOnScroll': 'Marcar como leído al desplazar',
    'settings.section.media': 'Multimedia',
    'settings.section.destinations': 'Destinos',
    'settings.section.data': 'Datos',
    'settings.section.diagnostics': 'Diagnóstico',
    'settings.language': 'Idioma',

    'import.run': 'Importar…',
    'opml.export': 'Exportar OPML…',
};
