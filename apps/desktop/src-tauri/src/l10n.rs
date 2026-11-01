//! The host's own strings, in the user's language.
//!
//! Two surfaces are drawn by the OS from Rust, outside the webview's i18n
//! catalogs: the background-refresh notification body ([`crate::notify`])
//! and the native menu bar ([`crate::menu`]). Both follow the same
//! `ui.locale` setting the frontend writes, through this small table —
//! one [`Strings`] per shipped locale, the same eight the frontend ships
//! (`apps/desktop/src/lib/i18n/locale.svelte.ts`). The wording follows each
//! catalog's terms (its "Add feed", "Starred", "Read later"…) and each
//! platform's menu conventions.

use curio_core::CoreHandle;

/// The settings key the frontend persists the chosen locale under.
pub const LOCALE_KEY: &str = "ui.locale";

/// A shipped locale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Locale {
    /// English — the default, and the fallback for an unknown id.
    #[default]
    En,
    /// Spanish.
    Es,
    /// French.
    Fr,
    /// German.
    De,
    /// Italian.
    It,
    /// Polish.
    Pl,
    /// Mainland Simplified Chinese.
    ZhHans,
    /// Cantonese, in Traditional script.
    Yue,
}

impl Locale {
    /// Every shipped locale.
    pub const ALL: [Self; 8] = [
        Self::En,
        Self::Es,
        Self::Fr,
        Self::De,
        Self::It,
        Self::Pl,
        Self::ZhHans,
        Self::Yue,
    ];

    /// The locale for a frontend locale id (`"de"`, `"zh-Hans"`), if it
    /// ships.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "en" => Self::En,
            "es" => Self::Es,
            "fr" => Self::Fr,
            "de" => Self::De,
            "it" => Self::It,
            "pl" => Self::Pl,
            "zh-Hans" => Self::ZhHans,
            "yue" => Self::Yue,
            _ => return None,
        })
    }

    /// The user's chosen locale, read fresh from settings; English when
    /// none is set (as in the frontend) or the setting cannot be read.
    #[must_use]
    pub fn from_settings(core: &CoreHandle) -> Self {
        core.get_setting(LOCALE_KEY)
            .ok()
            .flatten()
            .and_then(|id| Self::parse(&id))
            .unwrap_or_default()
    }

    /// This locale's strings.
    #[must_use]
    pub fn strings(self) -> &'static Strings {
        match self {
            Self::En => &EN,
            Self::Es => &ES,
            Self::Fr => &FR,
            Self::De => &DE,
            Self::It => &IT,
            Self::Pl => &PL,
            Self::ZhHans => &ZH_HANS,
            Self::Yue => &YUE,
        }
    }

    /// Picks the plural form for `count` from `[one, few, other]` (CLDR
    /// cardinal rules for integers, for the shipped locales: only Polish
    /// has a `few`; French counts 0 as `one`; Chinese has no plural).
    fn plural(self, count: u64, forms: &Forms) -> &'static str {
        let [one, few, other] = *forms;
        match self {
            Self::ZhHans | Self::Yue => other,
            Self::Fr if count <= 1 => one,
            Self::Pl => {
                let (tens, hundreds) = (count % 10, count % 100);
                if count == 1 {
                    one
                } else if (2..=4).contains(&tens) && !(12..=14).contains(&hundreds) {
                    few
                } else {
                    other
                }
            }
            _ if count == 1 => one,
            _ => other,
        }
    }

    /// `forms` for `count`, with `{n}` filled in.
    #[must_use]
    pub fn count(self, count: u64, forms: &Forms) -> String {
        self.plural(count, forms).replace("{n}", &count.to_string())
    }
}

/// A counted phrase by plural form: `[one, few, other]`, `{n}` the count.
/// `few` is only read for Polish; the other locales repeat `other` there.
pub type Forms = [&'static str; 3];

/// Every string the host draws itself, for one locale.
#[derive(Debug)]
pub struct Strings {
    /// Notification: new articles across the refreshed feeds.
    pub new_articles: Forms,
    /// Notification: feeds whose refresh failed.
    pub feeds_failed: Forms,
    /// Notification: feeds that went dead (HTTP 410).
    pub feeds_dead: Forms,
    /// The native menu bar.
    pub menu: MenuStrings,
}

/// The native menu's labels.
#[derive(Debug)]
#[allow(missing_docs)] // Each field is the label of the menu item it names.
pub struct MenuStrings {
    pub about: &'static str,
    pub settings: &'static str,
    pub hide: &'static str,
    pub hide_others: &'static str,
    pub show_all: &'static str,
    pub quit: &'static str,
    pub file: &'static str,
    pub add_feed: &'static str,
    pub edit: &'static str,
    pub undo: &'static str,
    pub redo: &'static str,
    pub cut: &'static str,
    pub copy: &'static str,
    pub paste: &'static str,
    pub select_all: &'static str,
    pub view: &'static str,
    pub refresh_all: &'static str,
    pub go: &'static str,
    pub all_articles: &'static str,
    pub starred: &'static str,
    pub read_later: &'static str,
    pub archived: &'static str,
    pub next_unread: &'static str,
    pub help: &'static str,
    pub shortcuts: &'static str,
    pub docs: &'static str,
    pub report_issue: &'static str,
}

static EN: Strings = Strings {
    new_articles: ["{n} new article", "{n} new articles", "{n} new articles"],
    feeds_failed: [
        "{n} feed failed to refresh",
        "{n} feeds failed to refresh",
        "{n} feeds failed to refresh",
    ],
    feeds_dead: [
        "{n} feed went dead",
        "{n} feeds went dead",
        "{n} feeds went dead",
    ],
    menu: MenuStrings {
        about: "About Curio",
        settings: "Settings…",
        hide: "Hide Curio",
        hide_others: "Hide Others",
        show_all: "Show All",
        quit: "Quit Curio",
        file: "File",
        add_feed: "Add Feed…",
        edit: "Edit",
        undo: "Undo",
        redo: "Redo",
        cut: "Cut",
        copy: "Copy",
        paste: "Paste",
        select_all: "Select All",
        view: "View",
        refresh_all: "Refresh All",
        go: "Go",
        all_articles: "All Articles",
        starred: "Starred",
        read_later: "Read Later",
        archived: "Archived",
        next_unread: "Next Unread",
        help: "Help",
        shortcuts: "Keyboard Shortcuts",
        docs: "Curio Docs",
        report_issue: "Report an Issue",
    },
};

static ES: Strings = Strings {
    new_articles: [
        "{n} artículo nuevo",
        "{n} artículos nuevos",
        "{n} artículos nuevos",
    ],
    feeds_failed: [
        "{n} feed no se pudo actualizar",
        "{n} feeds no se pudieron actualizar",
        "{n} feeds no se pudieron actualizar",
    ],
    feeds_dead: [
        "{n} feed dejó de funcionar",
        "{n} feeds dejaron de funcionar",
        "{n} feeds dejaron de funcionar",
    ],
    menu: MenuStrings {
        about: "Acerca de Curio",
        settings: "Ajustes…",
        hide: "Ocultar Curio",
        hide_others: "Ocultar otros",
        show_all: "Mostrar todo",
        quit: "Salir de Curio",
        file: "Archivo",
        add_feed: "Añadir feed…",
        edit: "Edición",
        undo: "Deshacer",
        redo: "Rehacer",
        cut: "Cortar",
        copy: "Copiar",
        paste: "Pegar",
        select_all: "Seleccionar todo",
        view: "Visualización",
        refresh_all: "Actualizar todo",
        go: "Ir",
        all_articles: "Todos los artículos",
        starred: "Destacados",
        read_later: "Leer más tarde",
        archived: "Archivados",
        next_unread: "Siguiente sin leer",
        help: "Ayuda",
        shortcuts: "Atajos de teclado",
        docs: "Documentación de Curio",
        report_issue: "Informar de un problema",
    },
};

static FR: Strings = Strings {
    new_articles: [
        "{n} nouvel article",
        "{n} nouveaux articles",
        "{n} nouveaux articles",
    ],
    feeds_failed: [
        "{n} flux n’a pas pu être actualisé",
        "{n} flux n’ont pas pu être actualisés",
        "{n} flux n’ont pas pu être actualisés",
    ],
    feeds_dead: [
        "{n} flux est mort",
        "{n} flux sont morts",
        "{n} flux sont morts",
    ],
    menu: MenuStrings {
        about: "À propos de Curio",
        settings: "Paramètres…",
        hide: "Masquer Curio",
        hide_others: "Masquer les autres",
        show_all: "Tout afficher",
        quit: "Quitter Curio",
        file: "Fichier",
        add_feed: "Ajouter un flux…",
        edit: "Édition",
        undo: "Annuler",
        redo: "Rétablir",
        cut: "Couper",
        copy: "Copier",
        paste: "Coller",
        select_all: "Tout sélectionner",
        view: "Présentation",
        refresh_all: "Tout actualiser",
        go: "Aller",
        all_articles: "Tous les articles",
        starred: "Favoris",
        read_later: "À lire plus tard",
        archived: "Archivés",
        next_unread: "Prochain non lu",
        help: "Aide",
        shortcuts: "Raccourcis clavier",
        docs: "Documentation de Curio",
        report_issue: "Signaler un problème",
    },
};

static DE: Strings = Strings {
    new_articles: ["{n} neuer Artikel", "{n} neue Artikel", "{n} neue Artikel"],
    feeds_failed: [
        "{n} Feed konnte nicht aktualisiert werden",
        "{n} Feeds konnten nicht aktualisiert werden",
        "{n} Feeds konnten nicht aktualisiert werden",
    ],
    feeds_dead: [
        "{n} Feed ist gestorben",
        "{n} Feeds sind gestorben",
        "{n} Feeds sind gestorben",
    ],
    menu: MenuStrings {
        about: "Über Curio",
        settings: "Einstellungen…",
        hide: "Curio ausblenden",
        hide_others: "Andere ausblenden",
        show_all: "Alle einblenden",
        quit: "Curio beenden",
        file: "Ablage",
        add_feed: "Feed hinzufügen…",
        edit: "Bearbeiten",
        undo: "Widerrufen",
        redo: "Wiederholen",
        cut: "Ausschneiden",
        copy: "Kopieren",
        paste: "Einsetzen",
        select_all: "Alles auswählen",
        view: "Darstellung",
        refresh_all: "Alle aktualisieren",
        go: "Gehe zu",
        all_articles: "Alle Artikel",
        starred: "Favoriten",
        read_later: "Später lesen",
        archived: "Archiviert",
        next_unread: "Nächster ungelesener",
        help: "Hilfe",
        shortcuts: "Tastenkürzel",
        docs: "Curio-Dokumentation",
        report_issue: "Problem melden",
    },
};

static IT: Strings = Strings {
    new_articles: [
        "{n} nuovo articolo",
        "{n} nuovi articoli",
        "{n} nuovi articoli",
    ],
    feeds_failed: [
        "{n} feed non si è aggiornato",
        "{n} feed non si sono aggiornati",
        "{n} feed non si sono aggiornati",
    ],
    feeds_dead: [
        "{n} feed è morto",
        "{n} feed sono morti",
        "{n} feed sono morti",
    ],
    menu: MenuStrings {
        about: "Informazioni su Curio",
        settings: "Impostazioni…",
        hide: "Nascondi Curio",
        hide_others: "Nascondi altre",
        show_all: "Mostra tutte",
        quit: "Esci da Curio",
        file: "File",
        add_feed: "Aggiungi feed…",
        edit: "Modifica",
        undo: "Annulla",
        redo: "Ripeti",
        cut: "Taglia",
        copy: "Copia",
        paste: "Incolla",
        select_all: "Seleziona tutto",
        view: "Vista",
        refresh_all: "Aggiorna tutto",
        go: "Vai",
        all_articles: "Tutti gli articoli",
        starred: "Preferiti",
        read_later: "Leggi dopo",
        archived: "Archiviati",
        next_unread: "Prossimo non letto",
        help: "Aiuto",
        shortcuts: "Scorciatoie da tastiera",
        docs: "Documentazione di Curio",
        report_issue: "Segnala un problema",
    },
};

static PL: Strings = Strings {
    new_articles: [
        "{n} nowy artykuł",
        "{n} nowe artykuły",
        "{n} nowych artykułów",
    ],
    feeds_failed: [
        "{n} kanał nie odświeżył się",
        "{n} kanały nie odświeżyły się",
        "{n} kanałów nie odświeżyło się",
    ],
    feeds_dead: [
        "{n} kanał przestał działać",
        "{n} kanały przestały działać",
        "{n} kanałów przestało działać",
    ],
    menu: MenuStrings {
        about: "O programie Curio",
        settings: "Ustawienia…",
        hide: "Ukryj Curio",
        hide_others: "Ukryj pozostałe",
        show_all: "Pokaż wszystkie",
        quit: "Zakończ Curio",
        file: "Plik",
        add_feed: "Dodaj kanał…",
        edit: "Edycja",
        undo: "Cofnij",
        redo: "Przywróć",
        cut: "Wytnij",
        copy: "Kopiuj",
        paste: "Wklej",
        select_all: "Zaznacz wszystko",
        view: "Widok",
        refresh_all: "Odśwież wszystko",
        go: "Idź",
        all_articles: "Wszystkie artykuły",
        starred: "Oznaczone gwiazdką",
        read_later: "Do przeczytania",
        archived: "Zarchiwizowane",
        next_unread: "Następny nieprzeczytany",
        help: "Pomoc",
        shortcuts: "Skróty klawiszowe",
        docs: "Dokumentacja Curio",
        report_issue: "Zgłoś problem",
    },
};

static ZH_HANS: Strings = Strings {
    new_articles: ["{n} 篇新文章", "{n} 篇新文章", "{n} 篇新文章"],
    feeds_failed: [
        "{n} 个订阅源刷新失败",
        "{n} 个订阅源刷新失败",
        "{n} 个订阅源刷新失败",
    ],
    feeds_dead: [
        "{n} 个订阅源已失效",
        "{n} 个订阅源已失效",
        "{n} 个订阅源已失效",
    ],
    menu: MenuStrings {
        about: "关于 Curio",
        settings: "设置…",
        hide: "隐藏 Curio",
        hide_others: "隐藏其他",
        show_all: "全部显示",
        quit: "退出 Curio",
        file: "文件",
        add_feed: "添加订阅源…",
        edit: "编辑",
        undo: "撤销",
        redo: "重做",
        cut: "剪切",
        copy: "拷贝",
        paste: "粘贴",
        select_all: "全选",
        view: "显示",
        refresh_all: "全部刷新",
        go: "前往",
        all_articles: "所有文章",
        starred: "星标",
        read_later: "稍后阅读",
        archived: "已归档",
        next_unread: "下一条未读",
        help: "帮助",
        shortcuts: "键盘快捷键",
        docs: "Curio 文档",
        report_issue: "报告问题",
    },
};

static YUE: Strings = Strings {
    new_articles: ["{n} 篇新文章", "{n} 篇新文章", "{n} 篇新文章"],
    feeds_failed: [
        "{n} 個訂閱源更新唔到",
        "{n} 個訂閱源更新唔到",
        "{n} 個訂閱源更新唔到",
    ],
    feeds_dead: [
        "{n} 個訂閱源已經失效",
        "{n} 個訂閱源已經失效",
        "{n} 個訂閱源已經失效",
    ],
    menu: MenuStrings {
        about: "關於 Curio",
        settings: "設定…",
        hide: "隱藏 Curio",
        hide_others: "隱藏其他",
        show_all: "全部顯示",
        quit: "結束 Curio",
        file: "檔案",
        add_feed: "加訂閱源…",
        edit: "編輯",
        undo: "還原",
        redo: "重做",
        cut: "剪下",
        copy: "拷貝",
        paste: "貼上",
        select_all: "全選",
        view: "顯示",
        refresh_all: "全部更新",
        go: "前往",
        all_articles: "全部文章",
        starred: "已加星標",
        read_later: "稍後睇",
        archived: "已封存",
        next_unread: "下一篇未睇",
        help: "說明",
        shortcuts: "鍵盤快捷鍵",
        docs: "Curio 說明文件",
        report_issue: "報告問題",
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Every label of one locale, for whole-table checks.
    fn labels(menu: &MenuStrings) -> [&'static str; 27] {
        [
            menu.about,
            menu.settings,
            menu.hide,
            menu.hide_others,
            menu.show_all,
            menu.quit,
            menu.file,
            menu.add_feed,
            menu.edit,
            menu.undo,
            menu.redo,
            menu.cut,
            menu.copy,
            menu.paste,
            menu.select_all,
            menu.view,
            menu.refresh_all,
            menu.go,
            menu.all_articles,
            menu.starred,
            menu.read_later,
            menu.archived,
            menu.next_unread,
            menu.help,
            menu.shortcuts,
            menu.docs,
            menu.report_issue,
        ]
    }

    #[test]
    fn the_frontend_locale_ids_parse_and_anything_else_is_english() {
        let ids = ["en", "es", "fr", "de", "it", "pl", "zh-Hans", "yue"];
        for (id, locale) in ids.iter().zip(Locale::ALL) {
            assert_eq!(Locale::parse(id), Some(locale));
        }
        assert_eq!(Locale::parse("zh-Hant"), None);
        assert_eq!(Locale::parse("DE"), None);
        assert_eq!(Locale::default(), Locale::En);
    }

    #[test]
    fn the_chosen_locale_is_read_from_settings() {
        let (_dir, core) = crate::commands::testutil::temp_core();
        assert_eq!(Locale::from_settings(&core), Locale::En);
        core.set_setting(LOCALE_KEY, "pl")
            .unwrap_or_else(|e| panic!("set: {e}"));
        assert_eq!(Locale::from_settings(&core), Locale::Pl);
        core.set_setting(LOCALE_KEY, "klingon")
            .unwrap_or_else(|e| panic!("set: {e}"));
        assert_eq!(Locale::from_settings(&core), Locale::En);
    }

    #[test]
    fn counts_pick_each_locales_plural_form() {
        let de = DE.new_articles;
        assert_eq!(Locale::De.count(1, &de), "1 neuer Artikel");
        assert_eq!(Locale::De.count(4, &de), "4 neue Artikel");
        // French counts zero as singular.
        assert_eq!(Locale::Fr.count(0, &FR.feeds_dead), "0 flux est mort");
        assert_eq!(Locale::Fr.count(2, &FR.feeds_dead), "2 flux sont morts");
        // Polish: one, few (2–4 but not 12–14), many.
        let pl = PL.new_articles;
        assert_eq!(Locale::Pl.count(1, &pl), "1 nowy artykuł");
        assert_eq!(Locale::Pl.count(3, &pl), "3 nowe artykuły");
        assert_eq!(Locale::Pl.count(22, &pl), "22 nowe artykuły");
        assert_eq!(Locale::Pl.count(12, &pl), "12 nowych artykułów");
        assert_eq!(Locale::Pl.count(5, &pl), "5 nowych artykułów");
        assert_eq!(Locale::ZhHans.count(1, &ZH_HANS.new_articles), "1 篇新文章");
    }

    #[test]
    fn every_locale_has_every_string_with_its_count() {
        for locale in Locale::ALL {
            let strings = locale.strings();
            for forms in [
                &strings.new_articles,
                &strings.feeds_failed,
                &strings.feeds_dead,
            ] {
                assert!(forms.iter().all(|form| form.contains("{n}")), "{locale:?}");
            }
            assert!(
                labels(&strings.menu).iter().all(|label| !label.is_empty()),
                "{locale:?}"
            );
        }
    }

    #[test]
    fn the_translated_menus_are_translated() {
        let english = labels(&EN.menu);
        for locale in Locale::ALL.into_iter().skip(1) {
            let translated = labels(&locale.strings().menu);
            let same = english
                .iter()
                .zip(translated)
                .filter(|(en, other)| *en == other)
                .count();
            // A loanword or two ("File" in Italian) may match; a table that
            // is mostly English is a copy-paste slip.
            assert!(same <= 2, "{locale:?} repeats {same} English labels");
        }
        assert_eq!(DE.menu.refresh_all, "Alle aktualisieren");
    }
}
