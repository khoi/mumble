//! UI translations for the port shells, keyed by the English source
//! string. The language follows the system locale unless a persisted
//! interface-language choice overrides it; unknown locales and
//! untranslated strings fall back to English. Composite strings are `{}`
//! templates filled through [`tr_fill`].

use std::collections::HashMap;
use std::sync::OnceLock;
#[cfg(not(test))]
use std::sync::atomic::{AtomicU8, Ordering};

mod french;
mod portuguese;
mod traditional_chinese;

/// A persisted interface-language code (`None` follows the system) and its
/// native name.
pub(crate) type LanguageChoice = (Option<&'static str>, &'static str);

/// The selectable interface languages: persisted code and native name. The
/// leading `None` entry follows the system display language. Português
/// (Brasil), Français, and 繁體中文 have not had a native speaker's review
/// yet.
pub(crate) const LANGUAGE_CHOICES: &[LanguageChoice] = &[
    (None, "System"),
    (Some("en"), "English"),
    (Some("pl"), "Polski"),
    (Some("zh"), "简体中文"),
    (Some("zh-Hant"), "繁體中文"),
    (Some("ja"), "日本語"),
    (Some("de"), "Deutsch"),
    (Some("es"), "Español"),
    (Some("uk"), "Українська"),
    (Some("pt"), "Português (Brasil)"),
    (Some("fr"), "Français"),
];

/// Active language column into [`TRANSLATIONS`]; 255 means English.
#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(255);

// Tests render scenes in different languages concurrently, so each test
// thread has its own language.
#[cfg(test)]
thread_local! {
    static CURRENT: std::cell::Cell<u8> = const { std::cell::Cell::new(ENGLISH) };
}

#[cfg(not(test))]
fn current() -> u8 {
    CURRENT.load(Ordering::Relaxed)
}

#[cfg(not(test))]
fn set_current(code: u8) {
    CURRENT.store(code, Ordering::Relaxed);
}

#[cfg(test)]
fn current() -> u8 {
    CURRENT.get()
}

#[cfg(test)]
fn set_current(code: u8) {
    CURRENT.set(code);
}

const ENGLISH: u8 = 255;
/// Brazilian Portuguese, kept in its own table ([`portuguese::TRANSLATIONS`]).
const PORTUGUESE_COLUMN: u8 = 6;
/// French, kept in its own table ([`french::TRANSLATIONS`]).
const FRENCH_COLUMN: u8 = 7;
/// Traditional Chinese, drawn from the Simplified column
/// ([`traditional_chinese::table`]).
const TRADITIONAL_CHINESE_COLUMN: u8 = 8;

/// The column for a persisted code; `pt` (any Portuguese system language)
/// is Brazilian Portuguese, and `fr` any French one.
fn column_for_code(code: &str) -> u8 {
    match code {
        "pl" => 0,
        "zh" => 1,
        "zh-Hant" => TRADITIONAL_CHINESE_COLUMN,
        "ja" => 2,
        "de" => 3,
        "es" => 4,
        "uk" => 5,
        "pt" => PORTUGUESE_COLUMN,
        "fr" => FRENCH_COLUMN,
        _ => ENGLISH,
    }
}

/// Resolve and activate the persisted language choice; `None` follows the
/// system locale. Safe to call again whenever the setting changes.
pub(crate) fn apply(setting: Option<&str>) {
    let code = match setting {
        Some(code) => column_for_code(code),
        None => column_for_language_tag(&preferred_locale()),
    };
    set_current(code);
}

/// The column for a system locale such as `zh-Hans-CN`, `zh_TW.UTF-8`, or
/// `pl`; unknown or empty tags stay English. Chinese with the Hant script,
/// or the Taiwan, Hong Kong, or Macau region, is Traditional Chinese.
fn column_for_language_tag(tag: &str) -> u8 {
    // POSIX locales carry an encoding and modifier: `zh_TW.UTF-8@euro`.
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    match primary_subtag(tag).as_deref() {
        Some("zh") if is_traditional_chinese(tag) => TRADITIONAL_CHINESE_COLUMN,
        Some(code) => column_for_code(code),
        None => ENGLISH,
    }
}

/// Whether a Chinese locale's script or region asks for Traditional
/// characters. An explicit script wins over the region (`zh-Hans-HK`).
fn is_traditional_chinese(tag: &str) -> bool {
    for subtag in tag.split(['-', '_']).skip(1) {
        match subtag.trim().to_ascii_lowercase().as_str() {
            "hant" | "tw" | "hk" | "mo" => return true,
            "hans" => return false,
            _ => {}
        }
    }
    false
}

/// The system locale with its script and region, for the interface
/// language: the Windows user locale or the POSIX locale variables.
fn system_locale() -> String {
    #[cfg(target_os = "windows")]
    {
        windows_user_locale()
    }
    #[cfg(not(target_os = "windows"))]
    {
        locale_from_env(
            std::env::var("LC_ALL").ok().as_deref(),
            std::env::var("LC_MESSAGES").ok().as_deref(),
            std::env::var("LANG").ok().as_deref(),
        )
    }
}

/// The person's first preferred locale in full (`zh-Hant-TW`), for the
/// interface language; on macOS the system's language list.
pub(crate) fn preferred_locale() -> String {
    #[cfg(target_os = "macos")]
    if let Some(locale) = macos_preferred_locale() {
        return locale;
    }
    system_locale()
}

/// The first entry of `NSLocale.preferredLanguages`, such as `zh-Hant-TW`.
#[cfg(target_os = "macos")]
fn macos_preferred_locale() -> Option<String> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2_foundation::{NSArray, NSString};

    let class = AnyClass::get(c"NSLocale")?;
    let languages: Option<Retained<NSArray<NSString>>> =
        unsafe { msg_send![class, preferredLanguages] };
    Some(languages?.firstObject()?.to_string())
}

/// `pt-BR` and `zh-Hans-CN` yield `pt` and `zh`.
fn primary_subtag(tag: &str) -> Option<String> {
    let subtag = tag.split(['-', '_']).next()?.trim().to_ascii_lowercase();
    (!subtag.is_empty()).then_some(subtag)
}

/// The Windows user locale name in full, such as `zh-TW`; `en` if unread.
#[cfg(target_os = "windows")]
fn windows_user_locale() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    const LOCALE_NAME_CAPACITY: usize = 85;
    let mut buffer = [0_u16; LOCALE_NAME_CAPACITY];
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    if length <= 1 {
        return "en".into();
    }
    String::from_utf16_lossy(&buffer[..length as usize - 1])
}

/// The first set POSIX locale variable in full, in POSIX precedence:
/// `zh_TW.UTF-8` stays `zh_TW.UTF-8`.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn locale_from_env(lc_all: Option<&str>, lc_messages: Option<&str>, lang: Option<&str>) -> String {
    [lc_all, lc_messages, lang]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty() && *value != "C" && *value != "POSIX")
        .unwrap_or_default()
        .to_owned()
}

/// Translate one UI string. Returns the input unchanged for English or for
/// strings missing from the table, so dynamic values pass through safely.
pub(crate) fn tr(english: &str) -> &str {
    let column = current();
    if column == ENGLISH {
        return english;
    }
    if column == TRADITIONAL_CHINESE_COLUMN {
        return match traditional_chinese::table().get(english) {
            Some(text) => text.as_str(),
            None => english,
        };
    }
    let table = match column {
        PORTUGUESE_COLUMN => portuguese::table(),
        FRENCH_COLUMN => french::table(),
        _ => &tables()[column as usize],
    };
    table.get(english).copied().unwrap_or(english)
}

/// Translate a `{}` template and substitute the dynamic value, keeping
/// language-specific word order intact.
pub(crate) fn tr_fill(template: &str, value: &str) -> String {
    tr(template).replace("{}", value)
}

/// Translate a template with named `{name}` placeholders and fill them, so
/// each language can order the values as its grammar needs.
pub(crate) fn tr_named(template: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(tr(template).to_string(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), value)
        })
}

/// A compact, translated age label for an event timestamp.
pub(crate) fn event_age(timestamp_ms: u64) -> String {
    let seconds = crate::history::now_ms().saturating_sub(timestamp_ms) / 1_000;
    let (template, value) = match seconds {
        0..=59 => ("{}s ago", seconds),
        60..=3_599 => ("{}m ago", seconds / 60),
        3_600..=86_399 => ("{}h ago", seconds / 3_600),
        _ => ("{}d ago", seconds / 86_400),
    };
    tr_fill(template, &value.to_string())
}

fn tables() -> &'static [HashMap<&'static str, &'static str>; 6] {
    static TABLES: OnceLock<[HashMap<&'static str, &'static str>; 6]> = OnceLock::new();
    TABLES.get_or_init(|| {
        std::array::from_fn(|column| {
            TRANSLATIONS
                .iter()
                .map(|(english, translated)| (*english, translated[column]))
                .collect()
        })
    })
}

/// Columns: Polish, Chinese (Simplified), Japanese, German, Spanish, Ukrainian.
#[rustfmt::skip]
const TRANSLATIONS: &[(&str, [&str; 6])] = &[
    ("Transcribe", ["Transkrybuj", "转写", "文字起こし", "Transkribieren", "Transcribir", "Розпізнати"]),
    ("Mumble is up to date", ["Mumble jest aktualny", "Mumble 已是最新版本", "Mumble は最新です", "Mumble ist auf dem neuesten Stand", "Mumble está actualizado", "Mumble має останню версію"]),
    ("Couldn't check for updates", ["Nie udało się sprawdzić aktualizacji", "无法检查更新", "アップデートを確認できませんでした", "Updates konnten nicht gesucht werden", "No se pudieron buscar actualizaciones", "Не вдалося перевірити оновлення"]),
        ("Hide menu bar icon", ["Ukryj ikonę na pasku menu", "隐藏菜单栏图标", "メニューバーのアイコンを隠す", "Menüleistensymbol ausblenden", "Ocultar el icono de la barra de menús", "Приховати значок у рядку меню"]),
    ("Hide Mumble's icon from the menu bar. Shortcuts keep working; open Mumble again from Finder or Spotlight for Settings", ["Ukrywa ikonę Mumble na pasku menu. Skróty nadal działają; otwórz Mumble ponownie z Findera lub Spotlight, aby przejść do Ustawień", "从菜单栏隐藏 Mumble 的图标。快捷键仍然有效；从访达或聚焦搜索再次打开 Mumble 即可进入设置", "メニューバーから Mumble のアイコンを隠します。ショートカットは引き続き使えます。設定を開くには Finder や Spotlight から Mumble をもう一度開きます", "Blendet das Mumble-Symbol in der Menüleiste aus. Kürzel funktionieren weiter; öffne Mumble erneut über den Finder oder Spotlight, um zu den Einstellungen zu gelangen", "Oculta el icono de Mumble en la barra de menús. Los atajos siguen funcionando; vuelve a abrir Mumble desde el Finder o Spotlight para ir a Ajustes", "Приховує значок Mumble у рядку меню. Комбінації клавіш і далі працюють; щоб відкрити Налаштування, знову відкрийте Mumble із Finder або Spotlight"]),
                ("Transcribe again", ["Transkrybuj ponownie", "重新转写", "もう一度文字起こし", "Erneut transkribieren", "Transcribir de nuevo", "Розпізнати знову"]),
                ("Replace", ["Zastąp", "替换", "置き換える", "Ersetzen", "Reemplazar", "Замінити"]),
                        ("Section key alone", ["Klawisz § bez modyfikatora", "单独使用 § 键", "§ キーを単独で使う", "§-Taste allein", "Tecla § sola", "Клавіша § окремо"]),
    ("Use the § key, left of 1 on ISO keyboards, as a shortcut without a modifier. Mumble takes § from every app while it's bound", ["Pozwala użyć klawisza § (na lewo od 1 na klawiaturach ISO) jako skrótu bez modyfikatora. Gdy jest przypisany, Mumble przejmuje § od wszystkich aplikacji", "将 § 键（ISO 键盘上 1 左侧的键）单独用作快捷键。设为快捷键期间，所有应用都收不到 § 键，由 Mumble 使用", "§ キー（ISO キーボードで 1 の左にあるキー）を修飾キーなしのショートカットとして使います。割り当てている間、Mumble はすべてのアプリから § を受け取ります", "Die §-Taste links neben der 1 auf ISO-Tastaturen ohne Zusatztaste als Kürzel verwenden. Solange sie zugewiesen ist, nimmt Mumble allen Apps die §-Taste weg", "Usa la tecla §, a la izquierda del 1 en teclados ISO, como atajo sin modificador. Mientras esté asignada, Mumble toma la tecla § de todas las apps", "Використовуйте клавішу § (ліворуч від 1 на клавіатурах ISO) як комбінацію без модифікатора. Поки її призначено, Mumble забирає § у всіх програм"]),
    ("Restore clipboard on GNOME", ["Przywracanie schowka w GNOME", "在 GNOME 上恢复剪贴板", "GNOME でクリップボードを復元", "Zwischenablage unter GNOME wiederherstellen", "Restaurar el portapapeles en GNOME", "Відновлення буфера обміну в GNOME"]),
    ("Put back what you copied before a dictation, through XWayland. Works only while XWayland is already running", ["Przywraca przez XWayland to, co było skopiowane przed dyktowaniem. Działa tylko, gdy XWayland już działa", "通过 XWayland 恢复听写前复制的内容。仅在 XWayland 已运行时有效", "ディクテーション前にコピーしていた内容を XWayland 経由で戻します。XWayland がすでに動作しているときだけ使えます", "Stellt über XWayland wieder her, was vor dem Diktat kopiert war. Funktioniert nur, wenn XWayland bereits läuft", "Devuelve lo que copiaste antes de dictar, a través de XWayland. Solo funciona si XWayland ya se está ejecutando", "Повертає через XWayland те, що було скопійовано перед диктуванням. Працює, лише коли XWayland уже запущено"]),
    ("This desktop doesn't let Mumble put your clipboard back after a paste. Turn on Labs › Restore clipboard on GNOME to try it.", ["To środowisko nie pozwala Mumble przywrócić schowka po wklejeniu. Aby spróbować, włącz Laboratorium › Przywracanie schowka w GNOME.", "此桌面不允许 Mumble 在粘贴后恢复剪贴板。要试用，请打开实验室 › 在 GNOME 上恢复剪贴板。", "このデスクトップでは、Mumble は貼り付け後にクリップボードを元に戻せません。試すには ラボ › GNOME でクリップボードを復元 をオンにしてください。", "Auf diesem Desktop kann Mumble die Zwischenablage nach dem Einfügen nicht wiederherstellen. Zum Ausprobieren Labs › Zwischenablage unter GNOME wiederherstellen einschalten.", "Este escritorio no deja que Mumble restaure tu portapapeles después de pegar. Para probarlo, activa Labs › Restaurar el portapapeles en GNOME.", "Це середовище не дає Mumble відновити буфер обміну після вставлення. Щоб спробувати, увімкніть Лабораторію › Відновлення буфера обміну в GNOME."]),
                        ("Fix results in Output", ["Poprawianie wyników w Wyjściu", "在输出中修正结果", "出力で結果を修正", "Ergebnisse in Ausgabe korrigieren", "Corregir resultados en Salida", "Виправлення результатів у Виведенні"]),
    ("Offer Vocabulary and Fix this beside Output's Try result, to correct a wrong result", ["Pokazuje Słownictwo i Popraw to obok wyniku Wypróbuj w Wyjściu, aby poprawić błędny wynik", "在输出的试用结果旁提供词汇和修正此项，用于纠正错误的结果", "出力の試用結果の横に語彙と「これを修正」を表示し、誤った結果を直せるようにします", "Bietet Vokabular und Korrigieren neben dem Testergebnis in Ausgabe an, um ein falsches Ergebnis zu berichtigen", "Ofrece Vocabulario y Corregir esto junto al resultado de Probar en Salida, para corregir un resultado erróneo", "Показує Словник і «Виправити це» біля результату спроби у Виведенні, щоб виправити хибний результат"]),
    ("Fix this", ["Popraw to", "修正此项", "これを修正", "Korrigieren", "Corregir esto", "Виправити це"]),
                ("Don't switch", ["Nie zmieniaj", "不切换", "切り替えない", "Nicht wechseln", "No cambiar", "Не перемикати"]),
    ("Your desktop assigns this key itself, so Mumble can't confirm it", ["Twój pulpit sam przypisuje ten klawisz, więc Mumble nie może tego potwierdzić", "你的桌面自行分配此按键，因此 Mumble 无法确认", "デスクトップがこのキーを独自に割り当てるため、Mumble では確認できません", "Dein Desktop weist diese Taste selbst zu, daher kann Mumble sie nicht bestätigen", "Tu escritorio asigna esta tecla por su cuenta, así que Mumble no puede confirmarla", "Ваша стільниця сама призначає цю клавішу, тож Mumble не може це підтвердити"]),
    ("Not installed", ["Nie zainstalowano", "未安装", "未インストール", "Nicht installiert", "No instalado", "Не встановлено"]),
            ("Shortcut side", ["Strona skrótu", "快捷键侧", "ショートカットの左右", "Seite des Kürzels", "Lado del atajo", "Бік комбінації клавіш"]),
    ("Labs", ["Laboratorium", "实验室", "ラボ", "Labs", "Labs", "Лабораторія"]),
    ("Tap or hold", ["Stuknij lub przytrzymaj", "轻点或按住", "タップまたは長押し", "Tippen oder halten", "Toca o mantén", "Торкніться або утримуйте"]),
    ("In Toggle mode, hold the shortcut to dictate until you let go; a short tap still starts and stops", ["W trybie przełączania przytrzymaj skrót, aby dyktować do puszczenia; krótkie stuknięcie nadal uruchamia i zatrzymuje", "在切换模式下，按住快捷键即可听写，松开即结束；轻点仍可开始和停止", "切り替えモードでショートカットを長押しすると、離すまで音声入力します。短いタップでは従来どおり開始と停止をします", "Im Umschaltmodus diktierst du, solange du das Kürzel hältst; kurzes Tippen startet und stoppt weiterhin", "En el modo alternar, mantén el atajo para dictar hasta soltarlo; un toque corto sigue iniciando y deteniendo", "У режимі перемикання утримуйте комбінацію, щоб диктувати, доки не відпустите; коротке торкання й далі запускає та зупиняє"]),
    ("Copy recent dictations", ["Kopiowanie ostatnich dyktowań", "复制最近的听写", "最近の音声入力をコピー", "Letzte Diktate kopieren", "Copiar dictados recientes", "Копіювання останніх диктувань"]),
    ("Copy one of your last five dictations from the menu bar, under Paste Last Dictation", ["Kopiuj jedno z pięciu ostatnich dyktowań z paska menu, pod Wklej ostatnie dyktowanie", "从菜单栏“粘贴上次听写”下方复制最近五条听写中的一条", "メニューバーの「最後の音声入力を貼り付け」の下から、直近 5 件の音声入力のいずれかをコピーします", "Eines deiner letzten fünf Diktate in der Menüleiste unter „Letztes Diktat einfügen“ kopieren", "Copia uno de tus últimos cinco dictados desde la barra de menús, bajo Pegar último dictado", "Копіюйте одне з п'яти останніх диктувань у рядку меню, під «Вставити останнє диктування»"]),
                ("Mumble Microphone virtual input and real-time voice filters", ["Wirtualne wejście Mumble Microphone i filtry głosu na żywo", "Mumble Microphone 虚拟输入与实时声音滤镜", "Mumble Microphone 仮想入力とリアルタイムのボイスフィルター", "Virtueller Eingang Mumble Microphone und Echtzeit-Stimmfilter", "Entrada virtual Mumble Microphone y filtros de voz en tiempo real", "Віртуальний вхід Mumble Microphone та голосові фільтри наживо"]),
    ("Live text", ["Tekst na żywo", "实时文字", "ライブテキスト", "Live-Text", "Texto en vivo", "Текст наживо"]),
    ("Show provisional words beside the HUD while dictating", ["Pokazuj robocze słowa obok HUD-u podczas dyktowania", "听写时在 HUD 旁显示临时词语", "音声入力中に HUD の横に仮の語句を表示します", "Vorläufige Wörter beim Diktieren neben dem HUD zeigen", "Muestra palabras provisionales junto al HUD al dictar", "Показувати проміжні слова поруч із HUD під час диктування"]),
    ("Scene and story browser in the window", ["Przeglądarka scen i makiet w oknie", "窗口内的场景与示例浏览器", "ウィンドウ内のシーンとストーリーのブラウザー", "Szenen- und Story-Browser im Fenster", "Explorador de escenas e historias en la ventana", "Переглядач сцен та історій у вікні"]),
        ("Try unfinished features. They may change or break.", ["Wypróbuj niedokończone funkcje. Mogą się zmienić lub przestać działać.", "试用尚未完成的功能。它们可能会变化或出错。", "未完成の機能を試せます。変更されたり、動かなくなったりすることがあります。", "Probiere unfertige Funktionen aus. Sie können sich ändern oder kaputtgehen.", "Prueba funciones sin terminar. Pueden cambiar o fallar.", "Спробуйте незавершені функції. Вони можуть змінитися або зламатися."]),
    ("On while the Mumble Microphone driver is installed. Uninstall the driver in Voice Filter to turn it off.", ["Włączony, dopóki zainstalowany jest sterownik Mumble Microphone. Odinstaluj sterownik w Filtrze głosu, aby go wyłączyć.", "安装 Mumble Microphone 驱动期间保持开启。在声音滤镜中卸载驱动即可关闭。", "Mumble Microphone ドライバーがインストールされている間はオンです。オフにするにはボイスフィルターでドライバーをアンインストールしてください。", "Eingeschaltet, solange der Mumble-Microphone-Treiber installiert ist. Deinstalliere den Treiber im Stimmfilter, um ihn auszuschalten.", "Activado mientras el controlador Mumble Microphone esté instalado. Desinstala el controlador en Filtro de voz para desactivarlo.", "Увімкнено, доки встановлено драйвер Mumble Microphone. Видаліть драйвер у Голосовому фільтрі, щоб вимкнути."]),
                ("Mouse shortcuts", ["Skróty myszy", "鼠标快捷键", "マウスショートカット", "Maus-Kürzel", "Atajos de ratón", "Комбінації з мишею"]),
    ("Record mouse buttons as shortcuts", ["Nagrywaj przyciski myszy jako skróty", "将鼠标按键录制为快捷键", "マウスボタンをショートカットとして記録します", "Maustasten als Kürzel aufnehmen", "Graba botones del ratón como atajos", "Записувати кнопки миші як комбінації"]),
    ("While dictating with Pause media: YouTube and other tabs, resumed only if Mumble paused them", ["Podczas dyktowania z Wstrzymaj multimedia: YouTube i inne karty, wznawiane tylko, jeśli wstrzymał je Mumble", "听写时搭配“暂停媒体”：YouTube 等标签页，仅在由 Mumble 暂停时才会恢复", "「メディアを一時停止」での音声入力中：YouTube などのタブ。Mumble が止めた場合だけ再開します", "Beim Diktieren mit „Medien pausieren“: YouTube und andere Tabs, nur fortgesetzt, wenn Mumble sie pausiert hat", "Al dictar con Pausar multimedia: YouTube y otras pestañas, reanudadas solo si Mumble las pausó", "Під час диктування з «Призупиняти медіа»: YouTube та інші вкладки, відновлюються, лише якщо їх призупинив Mumble"]),
    ("Mouse shortcuts are in Labs now, so your mouse-button shortcut went back to the default. Turn on Labs › Mouse shortcuts to use one again.", ["Skróty myszy są teraz w Laboratorium, więc skrót przycisku myszy wrócił do domyślnego. Włącz Laboratorium › Skróty myszy, aby znów go używać.", "鼠标快捷键已移至实验室，因此你的鼠标按键快捷键已恢复为默认值。开启实验室 › 鼠标快捷键即可再次使用。", "マウスショートカットはラボに移ったため、マウスボタンのショートカットは既定に戻りました。再び使うにはラボ › マウスショートカットをオンにしてください。", "Maus-Kürzel sind jetzt in Labs, daher wurde dein Maustasten-Kürzel auf den Standard zurückgesetzt. Schalte Labs › Maus-Kürzel ein, um wieder eines zu nutzen.", "Los atajos de ratón están ahora en Labs, así que tu atajo con botón del ratón volvió al predeterminado. Activa Labs › Atajos de ratón para volver a usar uno.", "Комбінації з мишею тепер у Лабораторії, тож вашу комбінацію з кнопкою миші повернуто до типової. Увімкніть Лабораторія › Комбінації з мишею, щоб знову її використати."]),
            ("Restore shortcut default", ["Przywracanie domyślnego skrótu", "恢复默认快捷键", "ショートカットを既定に戻す", "Standardkürzel wiederherstellen", "Restaurar atajo predeterminado", "Відновлення типової комбінації"]),
    ("Offer Restore default next to Paste last dictation's shortcut", ["Pokazuj Przywróć domyślny obok skrótu Wklej ostatnie dyktowanie", "在“粘贴上次听写”的快捷键旁提供“恢复默认”", "「最後の音声入力を貼り付け」のショートカットの横に「既定に戻す」を表示します", "Bietet „Standard wiederherstellen“ neben dem Kürzel für „Letztes Diktat einfügen“", "Ofrece Restaurar predeterminado junto al atajo de Pegar último dictado", "Пропонує «Відновити типову» поруч із комбінацією «Вставити останнє диктування»"]),
    ("Keep dictation on clipboard", ["Zostawiaj dyktowanie w schowku", "将听写保留在剪贴板", "音声入力をクリップボードに残す", "Diktat in der Zwischenablage lassen", "Mantener el dictado en el portapapeles", "Залишати диктування в буфері обміну"]),
    ("Leave the dictation on the clipboard after pasting instead of restoring what was there", ["Po wklejeniu zostawia dyktowanie w schowku zamiast przywracać poprzednią zawartość", "粘贴后将听写内容留在剪贴板，而不恢复原先的内容", "貼り付け後、元の内容を戻さずに音声入力をクリップボードに残します", "Lässt das Diktat nach dem Einfügen in der Zwischenablage, statt den vorherigen Inhalt wiederherzustellen", "Deja el dictado en el portapapeles después de pegar en lugar de restaurar lo que había", "Після вставлення залишає диктування в буфері обміну замість відновлення попереднього вмісту"]),
    ("Copy dictation only", ["Tylko kopiuj dyktowanie", "仅复制听写", "音声入力をコピーのみ", "Diktat nur kopieren", "Solo copiar el dictado", "Лише копіювати диктування"]),
    ("Put the dictation on the clipboard instead of pasting it", ["Umieszcza dyktowanie w schowku zamiast je wklejać", "将听写内容放到剪贴板，而不粘贴", "音声入力を貼り付けずにクリップボードに入れます", "Legt das Diktat in die Zwischenablage, statt es einzufügen", "Pone el dictado en el portapapeles en lugar de pegarlo", "Кладе диктування в буфер обміну замість вставлення"]),
    ("Spoken formatting", ["Formatowanie głosem", "语音格式命令", "音声での書式指定", "Gesprochene Formatierung", "Formato por voz", "Голосове форматування"]),
    ("Say \"new line\", \"new paragraph\", \"open quote\", or \"close quote\" to insert line breaks and quotation marks", ["Powiedz „new line”, „new paragraph”, „open quote” lub „close quote”, aby wstawić podział wiersza, akapit lub cudzysłów", "说“new line”“new paragraph”“open quote”或“close quote”即可插入换行和引号", "「new line」「new paragraph」「open quote」「close quote」と言うと、改行や引用符を入力します", "Sag „new line“, „new paragraph“, „open quote“ oder „close quote“, um Zeilenumbrüche und Anführungszeichen einzufügen", "Di «new line», «new paragraph», «open quote» o «close quote» para insertar saltos de línea y comillas", "Скажіть «new line», «new paragraph», «open quote» або «close quote», щоб вставити розриви рядків і лапки"]),
    ("Spoken symbols", ["Symbole głosem", "语音符号", "音声での記号入力", "Gesprochene Symbole", "Símbolos por voz", "Голосові символи"]),
    ("Say \"comma\", \"colon\", \"hashtag\", \"underscore\", or \"slash\" to insert the symbol joined to its words", ["Powiedz „comma”, „colon”, „hashtag”, „underscore” lub „slash”, aby wstawić symbol połączony z sąsiednimi słowami", "说“comma”“colon”“hashtag”“underscore”或“slash”即可插入与相邻词连接的符号", "「comma」「colon」「hashtag」「underscore」「slash」と言うと、前後の語につなげて記号を入力します", "Sag „comma“, „colon“, „hashtag“, „underscore“ oder „slash“, um das Symbol direkt an seine Wörter angefügt einzufügen", "Di «comma», «colon», «hashtag», «underscore» o «slash» para insertar el símbolo unido a sus palabras", "Скажіть «comma», «colon», «hashtag», «underscore» або «slash», щоб вставити символ, приєднаний до сусідніх слів"]),
    ("Remove filler words", ["Usuwaj wtrącenia", "移除口头填充词", "つなぎ言葉を削除", "Füllwörter entfernen", "Quitar muletillas", "Прибирати слова-паразити"]),
    ("Remove filler words such as um and uh from dictations, without an AI model", ["Usuwa z dyktowania wtrącenia takie jak „um” i „uh”, bez modelu AI", "从听写中移除“um”“uh”等填充词，无需 AI 模型", "AI モデルを使わずに、音声入力から「um」「uh」などのつなぎ言葉を削除します", "Entfernt Füllwörter wie „um“ und „uh“ aus Diktaten, ohne KI-Modell", "Quita muletillas como «um» y «uh» de los dictados, sin un modelo de IA", "Прибирає з диктувань слова-паразити на кшталт «um» і «uh», без моделі ШІ"]),
                                                                                                    ("Remove {}?", ["Usunąć {}?", "移除 {}？", "{} を削除しますか？", "{} entfernen?", "¿Quitar {}?", "Вилучити {}?"]),
                                ("Forget", ["Zapomnij", "忘记", "忘れる", "Vergessen", "Olvidar", "Забути"]),
                                            ("Unknown", ["Nieznana", "未知", "不明", "Unbekannt", "Desconocida", "Невідомо"]),
                ("Special keys alone", ["Klawisze specjalne bez modyfikatora", "单独使用特殊键", "特殊キーを単独で使う", "Sondertasten allein", "Teclas especiales solas", "Спеціальні клавіші окремо"]),
    ("Use Menu, Pause, or Scroll Lock as a shortcut without a modifier, such as Caps Lock remapped to Menu", ["Używaj klawisza Menu, Pause lub Scroll Lock jako skrótu bez modyfikatora, np. Caps Lock przemapowanego na Menu", "将 Menu、Pause 或 Scroll Lock 单独用作快捷键，例如重新映射为 Menu 的 Caps Lock", "Menu、Pause、Scroll Lock を修飾キーなしでショートカットに使います。Menu に割り当て直した Caps Lock など", "Menu, Pause oder Scroll Lock ohne Modifikator als Kürzel verwenden, etwa Caps Lock, das auf Menu umgelegt ist", "Usa Menu, Pause o Scroll Lock como atajo sin modificador, como Bloq Mayús reasignado a Menu", "Використовуйте Menu, Pause або Scroll Lock як комбінацію без модифікатора, наприклад Caps Lock, перепризначений на Menu"]),
    ("Dictation stats", ["Statystyki dyktowania", "听写统计", "音声入力の統計", "Diktatstatistik", "Estadísticas de dictado", "Статистика диктування"]),
    ("Show a summary of your retained dictations at the top of History, with mumble history stats", ["Pokazuje podsumowanie zachowanych dyktowań na górze Historii, także poleceniem mumble history stats", "在历史记录顶部显示已保留听写的摘要，也可使用 mumble history stats", "履歴の上部に保存済みの音声入力の概要を表示します。mumble history stats でも確認できます", "Zeigt oben im Verlauf eine Zusammenfassung deiner behaltenen Diktate, auch mit mumble history stats", "Muestra un resumen de tus dictados conservados en la parte superior del Historial, también con mumble history stats", "Показує підсумок збережених диктувань угорі Історії, а також командою mumble history stats"]),
    ("Dictations: {count} · Words: {words} · Words per minute: {wpm}", ["Dyktowania: {count} · Słowa: {words} · Słowa na minutę: {wpm}", "听写：{count} · 字词：{words} · 每分钟字词：{wpm}", "音声入力: {count} · 単語: {words} · 毎分の単語数: {wpm}", "Diktate: {count} · Wörter: {words} · Wörter pro Minute: {wpm}", "Dictados: {count} · Palabras: {words} · Palabras por minuto: {wpm}", "Диктувань: {count} · Слів: {words} · Слів за хвилину: {wpm}"]),
    ("Changed by Output: {percent}% · Daily streak: {days}", ["Zmienione przez Wyjście: {percent}% · Seria dni: {days}", "被输出更改：{percent}% · 连续天数：{days}", "出力による変更: {percent}% · 連続日数: {days}", "Von der Ausgabe geändert: {percent} % · Tage in Folge: {days}", "Cambiados por Salida: {percent} % · Días seguidos: {days}", "Змінено Виведенням: {percent}% · Днів поспіль: {days}"]),
    ("Top apps: {}", ["Najczęstsze aplikacje: {}", "常用应用：{}", "よく使うアプリ: {}", "Häufigste Apps: {}", "Apps principales: {}", "Найчастіші програми: {}"]),
                            ("Source", ["Źródło", "来源", "出典", "Quelle", "Fuente", "Джерело"]),
                                                                        ("Second dictation shortcut", ["Drugi skrót dyktowania", "第二个听写快捷键", "2つ目の音声入力ショートカット", "Zweites Diktierkürzel", "Segundo atajo de dictado", "Друга комбінація для диктування"]),
    ("Another shortcut that dictates like the main one, such as a key on an external keyboard", ["Kolejny skrót, który dyktuje jak główny, np. klawisz na zewnętrznej klawiaturze", "另一个与主快捷键一样听写的快捷键，例如外接键盘上的按键", "メインと同じように音声入力するもう1つのショートカット。外付けキーボードのキーなど", "Ein weiteres Kürzel, das wie das Hauptkürzel diktiert, etwa eine Taste auf einer externen Tastatur", "Otro atajo que dicta como el principal, como una tecla de un teclado externo", "Ще одна комбінація, що диктує як основна, наприклад клавіша на зовнішній клавіатурі"]),
    ("Smart spacing", ["Inteligentne odstępy", "智能空格", "スマートスペース", "Intelligente Leerzeichen", "Espaciado inteligente", "Розумні пробіли"]),
    ("Add a space before a dictation pasted right after a word", ["Dodaje spację przed dyktowaniem wklejonym tuż po słowie", "在紧接单词粘贴的听写前添加空格", "単語の直後に貼り付ける音声入力の前にスペースを入れます", "Fügt ein Leerzeichen vor einem Diktat ein, das direkt nach einem Wort eingefügt wird", "Añade un espacio antes de un dictado pegado justo después de una palabra", "Додає пробіл перед диктуванням, вставленим одразу після слова"]),
    ("Sided modifier chords", ["Skróty ze stronami modyfikatorów", "区分左右的修饰键组合", "左右を区別する修飾キーの組み合わせ", "Modifikator-Kombinationen mit Seite", "Combinaciones de modificadores con lado", "Комбінації модифікаторів із боком"]),
    ("Record which side each modifier of a shortcut is on, such as Right ⌘ + Right ⌥", ["Zapisuje, po której stronie jest każdy modyfikator skrótu, np. prawy ⌘ + prawy ⌥", "录制快捷键中每个修饰键位于哪一侧，例如右 ⌘ + 右 ⌥", "ショートカットの各修飾キーが左右どちらかを記録します。例: 右 ⌘ + 右 ⌥", "Nimmt auf, auf welcher Seite jeder Modifikator eines Kürzels liegt, etwa rechte ⌘ + rechte ⌥", "Graba en qué lado está cada modificador de un atajo, como ⌘ derecha + ⌥ derecha", "Записує, з якого боку кожен модифікатор комбінації, наприклад правий ⌘ + правий ⌥"]),
    ("Dictates like the shortcut above", ["Dyktuje jak skrót powyżej", "与上方快捷键一样听写", "上のショートカットと同じように音声入力します", "Diktiert wie das Kürzel oben", "Dicta como el atajo de arriba", "Диктує як комбінація вище"]),
    ("Discards the recording", ["Odrzuca nagranie", "丢弃当前录音", "録音を破棄します", "Verwirft die Aufnahme", "Descarta la grabación", "Відкидає запис"]),
    ("Bind mumble dictation cancel to a key in your desktop's keyboard shortcuts", ["Przypisz mumble dictation cancel do klawisza w skrótach klawiszowych pulpitu", "在桌面的键盘快捷键中为 mumble dictation cancel 绑定一个按键", "デスクトップのキーボードショートカットで mumble dictation cancel をキーに割り当ててください", "Lege mumble dictation cancel in den Tastenkürzeln deines Desktops auf eine Taste", "Asigna mumble dictation cancel a una tecla en los atajos de teclado de tu escritorio", "Призначте mumble dictation cancel клавішу в комбінаціях клавіш вашого робочого столу"]),
    ("Restore default shortcut", ["Przywróć domyślny skrót", "恢复默认快捷键", "既定のショートカットに戻す", "Standardkürzel wiederherstellen", "Restaurar atajo predeterminado", "Відновити типову комбінацію"]),
    ("Default shortcut: {}", ["Domyślny skrót: {}", "默认快捷键：{}", "既定のショートカット: {}", "Standardkürzel: {}", "Atajo predeterminado: {}", "Типова комбінація: {}"]),
        ("Cloud transcription", ["Transkrypcja w chmurze", "云端转写", "クラウド文字起こし", "Cloud-Transkription", "Transcripción en la nube", "Хмарна транскрипція"]),
        ("Transcribe with", ["Transkrybuj przez", "转写服务", "文字起こしに使用", "Transkribieren mit", "Transcribir con", "Транскрибувати через"]),
    ("Turn off the mic when not dictating", ["Wyłączaj mikrofon, gdy nie dyktujesz", "不听写时关闭麦克风", "音声入力していないときはマイクをオフにする", "Mikrofon ausschalten, wenn du nicht diktierst", "Apagar el micrófono cuando no dictas", "Вимикати мікрофон, коли не диктуєте"]),
    ("The mic opens only while you dictate; the first moment can be clipped", ["Mikrofon otwiera się tylko podczas dyktowania; pierwsza chwila może zostać ucięta", "麦克风只在听写时打开；开头一瞬间可能被截掉", "マイクは音声入力中だけ開きます。最初の一瞬が切れることがあります", "Das Mikrofon öffnet sich nur beim Diktieren; der erste Moment kann abgeschnitten werden", "El micrófono solo se abre mientras dictas; el primer instante puede cortarse", "Мікрофон відкривається лише під час диктування; перша мить може обрізатися"]),
    ("The mic stays ready so dictation starts instantly; your system shows it in use", ["Mikrofon jest stale gotowy, więc dyktowanie zaczyna się od razu; system pokazuje, że jest używany", "麦克风保持就绪，听写可立即开始；系统会显示它正在使用", "マイクを待機させておくので音声入力がすぐ始まります。システムには使用中と表示されます", "Das Mikrofon bleibt bereit, damit das Diktat sofort beginnt; dein System zeigt es als benutzt an", "El micrófono queda listo para que el dictado empiece al instante; tu sistema lo muestra en uso", "Мікрофон лишається готовим, тож диктування починається миттєво; система показує, що він використовується"]),
    ("Send feedback", ["Wyślij opinię", "发送反馈", "フィードバックを送信", "Feedback senden", "Enviar comentarios", "Надіслати відгук"]),
    ("Run setup again", ["Uruchom konfigurację ponownie", "重新运行设置", "セットアップをもう一度実行", "Einrichtung erneut ausführen", "Volver a ejecutar la configuración", "Запустити налаштування знову"]),
    ("Play the first run again. Settings, models, and permissions stay", ["Przejdź pierwsze uruchomienie jeszcze raz. Ustawienia, modele i uprawnienia zostają", "再次体验首次运行。设置、模型和权限保持不变", "初回起動をもう一度行います。設定、モデル、権限はそのままです", "Den ersten Start noch einmal durchlaufen. Einstellungen, Modelle und Berechtigungen bleiben", "Repite el primer inicio. La configuración, los modelos y los permisos se conservan", "Пройти перший запуск ще раз. Налаштування, моделі й дозволи залишаться"]),
    ("Run", ["Uruchom", "运行", "実行", "Ausführen", "Ejecutar", "Запустити"]),
                ("Add application", ["Dodaj aplikację", "添加应用", "アプリを追加", "App hinzufügen", "Añadir aplicación", "Додати програму"]),
    ("Add transformation", ["Dodaj transformację", "添加转换", "変換を追加", "Transformation hinzufügen", "Añadir transformación", "Додати перетворення"]),
    ("Close", ["Zamknij", "关闭", "閉じる", "Schließen", "Cerrar", "Закрити"]),
    ("All files", ["Wszystkie pliki", "所有文件", "すべてのファイル", "Alle Dateien", "Todos los archivos", "Усі файли"]),
    ("Reorder", ["Zmień kolejność", "重新排序", "並べ替え", "Neu anordnen", "Reordenar", "Змінити порядок"]),
    ("No models match", ["Brak pasujących modeli", "没有匹配的模型", "一致するモデルはありません", "Keine passenden Modelle", "Ningún modelo coincide", "Відповідних моделей немає"]),
    ("Status", ["Stan", "状态", "ステータス", "Status", "Estado", "Стан"]),
    ("Connecting…", ["Łączenie…", "正在连接…", "接続中…", "Verbindung wird hergestellt…", "Conectando…", "Підключення…"]),
    ("Esc to cancel", ["Esc, aby anulować", "按 Esc 取消", "Esc でキャンセル", "Esc zum Abbrechen", "Esc para cancelar", "Esc, щоб скасувати"]),
    ("Desktop shortcuts", ["Skróty pulpitu", "桌面快捷键", "デスクトップのショートカット", "Desktop-Tastenkürzel", "Atajos del escritorio", "Системні комбінації клавіш"]),
    ("Change the keys your desktop gave Mumble", ["Zmień klawisze, które pulpit przydzielił Mumble", "更改桌面分配给 Mumble 的按键", "デスクトップが Mumble に割り当てたキーを変更", "Tasten ändern, die der Desktop Mumble zugewiesen hat", "Cambia las teclas que el escritorio asignó a Mumble", "Змінити клавіші, які система призначила Mumble"]),
        ("Your desktop granted {}", ["Pulpit przyznał {}", "桌面已授予 {}", "デスクトップが {} を割り当て", "Vom Desktop vergeben: {}", "Tu escritorio concedió {}", "Система надала {}"]),
    ("Your desktop did not grant this shortcut", ["Pulpit nie przyznał tego skrótu", "桌面未授予此快捷键", "デスクトップはこのショートカットを割り当てませんでした", "Der Desktop hat dieses Tastenkürzel nicht vergeben", "Tu escritorio no concedió este atajo", "Система не надала цю комбінацію клавіш"]),
    ("GNOME keyboard shortcuts", ["Skróty klawiszowe GNOME", "GNOME 键盘快捷键", "GNOME のキーボードショートカット", "GNOME-Tastaturkürzel", "Atajos de teclado de GNOME", "Комбінації клавіш GNOME"]),
    ("Add GNOME keyboard shortcuts", ["Dodaj skróty klawiszowe GNOME", "添加 GNOME 键盘快捷键", "GNOME のキーボードショートカットを追加", "GNOME-Tastaturkürzel hinzufügen", "Añadir atajos de teclado de GNOME", "Додати комбінації клавіш GNOME"]),
    ("Adds dictation and paste last to GNOME Settings › Keyboard › Custom Shortcuts with your shortcuts. GNOME runs them on the key press, so dictation toggles.", ["Dodaje dyktowanie i wklejanie ostatniego do Ustawień GNOME › Klawiatura › Własne skróty z Twoimi skrótami. GNOME uruchamia je przy naciśnięciu klawisza, więc dyktowanie się przełącza.", "将听写和粘贴上次以你的快捷键添加到 GNOME 设置 › 键盘 › 自定义快捷键。GNOME 在按下按键时运行它们，因此听写为切换方式。", "音声入力と前回を貼り付けをあなたのショートカットで GNOME 設定 › キーボード › カスタムショートカットに追加します。GNOME はキーを押したときに実行するため、音声入力は切り替え式になります。", "Fügt Diktat und Letztes einfügen mit deinen Tastenkürzeln zu GNOME-Einstellungen › Tastatur › Eigene Tastenkürzel hinzu. GNOME führt sie beim Tastendruck aus, daher schaltet das Diktat um.", "Añade dictado y pegar el último a Configuración de GNOME › Teclado › Atajos personalizados con tus atajos. GNOME los ejecuta al pulsar la tecla, así que el dictado se alterna.", "Додає диктування та вставлення останнього до Налаштувань GNOME › Клавіатура › Власні комбінації з вашими комбінаціями. GNOME запускає їх під час натискання клавіші, тож диктування перемикається."]),
    ("Added to GNOME Settings › Keyboard › Custom Shortcuts while Mumble is enabled. GNOME runs them on the key press, so your dictation shortcut toggles dictation. Change the keys there.", ["Dodane do Ustawień GNOME › Klawiatura › Własne skróty, gdy Mumble jest włączony. GNOME uruchamia je przy naciśnięciu klawisza, więc skrót dyktowania je przełącza. Klawisze zmienisz tam.", "Mumble 启用时已添加到 GNOME 设置 › 键盘 › 自定义快捷键。GNOME 在按下按键时运行它们，因此听写快捷键会切换听写。可在那里更改按键。", "Mumble が有効な間、GNOME 設定 › キーボード › カスタムショートカットに追加されます。GNOME はキーを押したときに実行するため、音声入力のショートカットは切り替え式です。キーはそこで変更できます。", "Zu GNOME-Einstellungen › Tastatur › Eigene Tastenkürzel hinzugefügt, solange Mumble aktiv ist. GNOME führt sie beim Tastendruck aus, daher schaltet dein Diktat-Kürzel das Diktat um. Ändere die Tasten dort.", "Añadidos a Configuración de GNOME › Teclado › Atajos personalizados mientras Mumble está activado. GNOME los ejecuta al pulsar la tecla, así que tu atajo de dictado lo alterna. Cambia las teclas allí.", "Додано до Налаштувань GNOME › Клавіатура › Власні комбінації, поки Mumble увімкнено. GNOME запускає їх під час натискання клавіші, тож ваша комбінація диктування перемикає його. Змініть клавіші там."]),
    ("GNOME uses {} itself, so it never reaches Mumble. Record another shortcut, or change it in GNOME Settings › Keyboard › Keyboard Shortcuts.", ["GNOME używa {} do własnych celów, więc nie dociera on do Mumble. Nagraj inny skrót albo zmień go w Ustawieniach GNOME › Klawiatura › Skróty klawiszowe.", "GNOME 自己使用 {}，所以它不会传到 Mumble。请录制其他快捷键，或在 GNOME 设置 › 键盘 › 键盘快捷键中更改。", "GNOME が {} を使用しているため、Mumble に届きません。別のショートカットを記録するか、GNOME 設定 › キーボード › キーボードショートカットで変更してください。", "GNOME verwendet {} selbst, daher erreicht es Mumble nie. Nimm ein anderes Tastenkürzel auf oder ändere es in den GNOME-Einstellungen › Tastatur › Tastaturkürzel.", "GNOME usa {} para sí mismo, así que nunca llega a Mumble. Graba otro atajo o cámbialo en Configuración de GNOME › Teclado › Atajos de teclado.", "GNOME використовує {} сам, тому вона не доходить до Mumble. Запишіть іншу комбінацію або змініть її в Налаштуваннях GNOME › Клавіатура › Комбінації клавіш."]),
    ("Your desktop kept a shortcut for itself. Record another one, or free it in your desktop's keyboard settings (on niri, remove its bind from config.kdl).", ["Pulpit zachował skrót dla siebie. Nagraj inny albo zwolnij go w ustawieniach klawiatury pulpitu (w niri usuń jego przypisanie z config.kdl).", "桌面保留了某个快捷键。请录制其他快捷键，或在桌面的键盘设置中释放它（在 niri 中，从 config.kdl 删除其绑定）。", "デスクトップがショートカットを使用中です。別のショートカットを記録するか、デスクトップのキーボード設定で空けてください（niri では config.kdl からその割り当てを削除）。", "Der Desktop behält ein Tastenkürzel für sich. Nimm ein anderes auf oder gib es in den Tastatureinstellungen des Desktops frei (unter niri die Belegung aus config.kdl entfernen).", "Tu escritorio se reservó un atajo. Graba otro o libéralo en los ajustes de teclado del escritorio (en niri, quita su asignación de config.kdl).", "Система залишила комбінацію клавіш за собою. Запишіть іншу або звільніть її в налаштуваннях клавіатури системи (у niri видаліть її призначення з config.kdl)."]),
            ("Your desktop has no shortcuts portal. Bind these commands in its keyboard settings.", ["Pulpit nie ma portalu skrótów. Przypisz te polecenia w jego ustawieniach klawiatury.", "你的桌面没有快捷键门户。请在其键盘设置中绑定这些命令。", "デスクトップにショートカットポータルがありません。キーボード設定でこれらのコマンドを割り当ててください。", "Dein Desktop hat kein Tastenkürzel-Portal. Weise diese Befehle in seinen Tastatureinstellungen zu.", "Tu escritorio no tiene portal de atajos. Asigna estos comandos en sus ajustes de teclado.", "У вашій системі немає порталу комбінацій клавіш. Призначте ці команди в її налаштуваннях клавіатури."]),
    ("Hold to dictate", ["Przytrzymaj, aby dyktować", "按住听写", "押している間音声入力", "Halten zum Diktieren", "Mantén pulsado para dictar", "Утримуйте, щоб диктувати"]),
    ("Bind start to the key press and stop to its release", ["Przypisz start do naciśnięcia klawisza, a stop do jego zwolnienia", "将 start 绑定到按下按键，将 stop 绑定到松开按键", "start をキーを押したとき、stop を離したときに割り当て", "start an das Drücken und stop an das Loslassen der Taste binden", "Asigna start al pulsar la tecla y stop al soltarla", "Привʼяжіть start до натискання клавіші, а stop до її відпускання"]),
    ("Toggle dictation", ["Przełącz dyktowanie", "切换听写", "音声入力を切り替え", "Diktat umschalten", "Alternar dictado", "Перемкнути диктування"]),
    ("Ready anywhere · Use your desktop's shortcuts to dictate", ["Gotowe wszędzie · Dyktuj skrótami pulpitu", "随处可用 · 使用桌面快捷键听写", "どこでも利用可能 · デスクトップのショートカットで音声入力", "Überall bereit · Diktiere mit den Tastenkürzeln des Desktops", "Listo en cualquier lugar · Dicta con los atajos del escritorio", "Готово всюди · Диктуйте за допомогою системних комбінацій клавіш"]),
    ("Interface language", ["Język interfejsu", "界面语言", "表示言語", "Sprache der Oberfläche", "Idioma de la interfaz", "Мова інтерфейсу"]),
        ("System", ["Systemowy", "跟随系统", "システム", "System", "Sistema", "Системна"]),
                ("Use", ["Użyj", "使用", "使用", "Verwenden", "Usar", "Вибрати"]),
    ("Download", ["Pobierz", "下载", "ダウンロード", "Herunterladen", "Descargar", "Завантажити"]),
    ("Cancel", ["Anuluj", "取消", "キャンセル", "Abbrechen", "Cancelar", "Скасувати"]),
    ("Active", ["Aktywny", "使用中", "有効", "Aktiv", "Activo", "Активна"]),
    ("Auto", ["Auto", "自动", "自動", "Auto", "Auto", "Авто"]),
    ("Downloading", ["Pobieranie", "下载中", "ダウンロード中", "Lädt herunter", "Descargando", "Завантаження"]),
                ("Recommended", ["Zalecany", "推荐", "おすすめ", "Empfohlen", "Recomendado", "Рекомендована"]),
    ("General", ["Ogólne", "通用", "一般", "Allgemein", "General", "Загальні"]),
    ("Experiments", ["Eksperymenty", "实验", "実験", "Experimente", "Experimentos", "Експерименти"]),
    ("Voice Filter", ["Filtr głosu", "声音滤镜", "ボイスフィルター", "Stimmfilter", "Filtro de voz", "Голосовий фільтр"]),
    ("Advanced", ["Zaawansowane", "高级", "詳細", "Erweitert", "Avanzado", "Додатково"]),
    ("Output", ["Wyjście", "输出", "出力", "Ausgabe", "Salida", "Виведення"]),
    ("GPU", ["GPU", "GPU", "GPU", "GPU", "GPU", "GPU"]),
    ("CPU", ["CPU", "CPU", "CPU", "CPU", "CPU", "CPU"]),
                ("Dictation HUD", ["Wskaźnik dyktowania", "听写指示器", "音声入力インジケーター", "Diktatanzeige", "Indicador de dictado", "HUD диктування"]),
        ("Check for updates", ["Sprawdź aktualizacje", "检查更新", "アップデートを確認", "Nach Updates suchen", "Buscar actualizaciones", "Перевірити оновлення"]),
    ("Checking for updates…", ["Sprawdzanie aktualizacji…", "正在检查更新…", "アップデートを確認中…", "Suche nach Updates…", "Buscando actualizaciones…", "Перевірка оновлень…"]),
    ("Start listening when Mumble opens", ["Zacznij nasłuchiwać po otwarciu Mumble", "Mumble 打开时开始监听", "Mumble の起動時に待ち受けを開始", "Beim Öffnen von Mumble zuhören", "Empezar a escuchar al abrir Mumble", "Починати слухати, коли Mumble відкривається"]),
                ("Modes", ["Tryby", "模式", "モード", "Modi", "Modos", "Режими"]),
    ("History", ["Historia", "历史记录", "履歴", "Verlauf", "Historial", "Історія"]),
    ("Add mode", ["Dodaj tryb", "添加模式", "モードを追加", "Modus hinzufügen", "Añadir modo", "Додати режим"]),
    ("Add correction", ["Dodaj poprawkę", "添加更正", "修正を追加", "Korrektur hinzufügen", "Añadir corrección", "Додати виправлення"]),
    ("Vocabulary", ["Słownictwo", "词汇表", "語彙", "Vokabular", "Vocabulario", "Словник"]),
        ("Transcription and rewriting spell them right, and search finds them when misheard", ["Transkrypcja i przepisywanie zapisują je poprawnie, a wyszukiwanie znajduje je, gdy zostaną źle usłyszane", "转写和改写时拼写正确，听错时搜索也能找到", "文字起こしと書き直しで正しく綴られ、聞き間違えられても検索で見つかります", "Transkription und Umschreiben schreiben sie richtig, und die Suche findet sie auch falsch verstanden", "La transcripción y la reescritura los escriben bien y la búsqueda los encuentra aunque se oigan mal", "Транскрипція й переписування пишуть їх правильно, а пошук знаходить їх, навіть коли їх почуто неправильно"]),
    ("Terms with spaces match only when said in full", ["Terminy ze spacjami pasują tylko wypowiedziane w całości", "带空格的术语只有完整说出时才会匹配", "スペースを含む用語は、全体を言ったときだけ一致します", "Begriffe mit Leerzeichen greifen nur, wenn sie ganz gesagt werden", "Los términos con espacios solo coinciden si se dicen completos", "Терміни з пробілами збігаються, лише коли їх сказано повністю"]),
    ("Light", ["Lekkie", "轻", "弱", "Leicht", "Ligero", "Слабке"]),
        ("Add term", ["Dodaj termin", "添加术语", "用語を追加", "Begriff hinzufügen", "Añadir término", "Додати термін"]),
    ("1 term", ["1 termin", "1 个术语", "1 件の用語", "1 Begriff", "1 término", "1 термін"]),
    ("{} terms", ["Terminy: {}", "{} 个术语", "{} 件の用語", "{} Begriffe", "{} términos", "Термінів: {}"]),
    ("Term (e.g. cappuccino)", ["Termin (np. cappuccino)", "术语（如 cappuccino）", "用語（例: cappuccino）", "Begriff (z. B. cappuccino)", "Término (p. ej., cappuccino)", "Термін (напр., cappuccino)"]),
    ("Sounds like (e.g. capuchin, cap a chino)", ["Brzmi jak (np. capuchin, cap a chino)", "听起来像（如 capuchin, cap a chino）", "聞こえ方（例: capuchin, cap a chino）", "Klingt wie (z. B. capuchin, cap a chino)", "Suena como (p. ej., capuchin, cap a chino)", "Звучить як (напр., capuchin, cap a chino)"]),
    ("Up to 16 spellings per term are saved, and 64 characters per term or spelling.", ["Zapisywanych jest do 16 pisowni na termin i do 64 znaków na termin lub pisownię.", "每个术语最多保存 16 种拼写，每个术语或拼写最多 64 个字符。", "保存されるのは用語ごとに最大 16 個の表記、用語や表記ごとに最大 64 文字です。", "Gespeichert werden bis zu 16 Schreibweisen pro Begriff und 64 Zeichen pro Begriff oder Schreibweise.", "Se guardan hasta 16 grafías por término y 64 caracteres por término o grafía.", "Зберігається до 16 написань для кожного терміна й до 64 символів у терміні чи написанні."]),
    ("Clear all", ["Wyczyść wszystko", "全部清除", "すべて消去", "Alle löschen", "Borrar todo", "Очистити все"]),
    ("Really clear all?", ["Na pewno wyczyścić?", "确定全部清除？", "本当にすべて消去？", "Wirklich alle löschen?", "¿Borrar todo?", "Справді очистити все?"]),
    ("Copy", ["Kopiuj", "复制", "コピー", "Kopieren", "Copiar", "Копіювати"]),
    ("(nothing)", ["(nic)", "（无）", "（なし）", "(nichts)", "(nada)", "(нічого)"]),
    ("Replace words", ["Zamień słowa", "替换词语", "単語を置き換え", "Wörter ersetzen", "Reemplazar palabras", "Замінити слова"]),
    ("Rewrite with OpenCode", ["Przepisz z OpenCode", "用 OpenCode 改写", "OpenCode で書き直す", "Mit OpenCode umschreiben", "Reescribir con OpenCode", "Переписати з OpenCode"]),
    ("Lowercase", ["Małe litery", "小写", "小文字", "Kleinbuchstaben", "Minúsculas", "Малі літери"]),
    ("Lowercases everything", ["Zmienia wszystko na małe litery", "全部转为小写", "すべて小文字にします", "Schreibt alles klein", "Pone todo en minúsculas", "Робить усе малими літерами"]),
    ("No final period", ["Bez kropki na końcu", "去掉末尾句号", "最後の句点なし", "Kein Schlusspunkt", "Sin punto final", "Без крапки в кінці"]),
    ("Drops the last . ! or ?", ["Usuwa ostatnią . ! lub ?", "去掉最后的 . ! 或 ?", "最後の . ! ? を削除します", "Entfernt das letzte . ! oder ?", "Quita el último . ! o ?", "Прибирає останню . ! чи ?"]),
    ("SpongeBob case", ["Styl SpongeBoba", "海绵宝宝大小写", "スポンジ・ボブ表記", "SpongeBob-Schreibweise", "Estilo Bob Esponja", "Стиль Губки Боба"]),
    ("aLtErNaTeS every letter", ["nAp RzEmIeNnIe litery", "字母大小写交替", "文字を交互に大小にします", "wEcHsElT jeden Buchstaben", "aLtErNa cada letra", "чЕрГуЄ кожну літеру"]),
    ("Copied. Paste it into your agent and say what you want", ["Skopiowano. Wklej to swojemu agentowi i powiedz, czego chcesz", "已复制。粘贴给你的智能体，说出你想要的", "コピーしました。エージェントに貼り付けて、望むことを伝えてください", "Kopiert. Füge es bei deinem Agenten ein und sag, was du willst", "Copiado. Pégalo en tu agente y di lo que quieres", "Скопійовано. Вставте це своєму агентові й скажіть, чого хочете"]),
    ("Tell your agent how Mumble should write", ["Powiedz agentowi, jak Mumble ma pisać", "告诉你的智能体 Mumble 该怎么写", "Mumble の書き方をエージェントに伝えましょう", "Sag deinem Agenten, wie Mumble schreiben soll", "Dile a tu agente cómo debe escribir Mumble", "Скажіть агенту, як має писати Mumble"]),
    ("Use all lowercase in my messaging apps", ["Pisz małymi literami w komunikatorach", "在聊天应用里全部用小写", "メッセージアプリではすべて小文字で", "Schreib in Messengern alles klein", "Usa todo en minúsculas en mis apps de mensajes", "Пиши малими літерами в месенджерах"]),
    ("Use all caps when I sound excited", ["Pisz wielkimi literami, gdy brzmię na podekscytowanego", "我一激动就全部大写", "興奮しているときはすべて大文字で", "Schreib alles groß, wenn ich aufgeregt klinge", "Usa mayúsculas cuando suene emocionado", "Пиши великими літерами, коли я схвильований"]),
    ("Use SpongeBob case when I’m being sarcastic", ["Pisz jak SpongeBob, gdy jestem sarkastyczny", "我阴阳怪气时用海绵宝宝体", "皮肉を言うときはスポンジ・ボブ書きで", "Schreib im SpongeBob-Stil, wenn ich sarkastisch bin", "Usa el estilo de Bob Esponja cuando sea sarcástico", "Пиши як Губка Боб, коли я саркастичний"]),
    ("Use Title Case for my email subjects", ["Każde Słowo Wielką Literą w tematach maili", "邮件主题每个单词首字母大写", "メールの件名は単語の頭を大文字に", "Jedes Wort groß in Betreffzeilen", "Cada Palabra En Mayúscula en los asuntos de correo", "Кожне Слово З Великої в темах листів"]),
    ("Spell it “Kubernetes”, not “Cooper Netties”", ["Pisz „Kubernetes”, a nie „Cooper Netties”", "要写“Kubernetes”，不是“Cooper Netties”", "「Cooper Netties」ではなく「Kubernetes」と書いて", "Schreib „Kubernetes“, nicht „Cooper Netties“", "Escribe «Kubernetes», no «Cooper Netties»", "Пиши «Kubernetes», а не «Cooper Netties»"]),
    ("Delete every “um” and “like”", ["Usuwaj każde „yyy” i „no wiesz”", "删掉所有的“嗯”和“就是”", "「えーと」と「なんか」は全部消して", "Streich jedes „äh“ und „halt“", "Borra cada «eh» y «o sea»", "Прибирай усі «е-е» і «типу»"]),
    ("Turn “let’s circle back” into “no”", ["Zamieniaj „wrócimy do tego” na „nie”", "把“我们回头再说”改成“不行”", "「また今度検討します」は「無理です」に", "Mach aus „Lass uns das später aufgreifen“ einfach „Nein“", "Convierte «lo retomamos luego» en «no»", "Заміняй «повернемося до цього» на «ні»"]),
    ("Talk like a pirate on Fridays", ["W piątki mów jak pirat", "每周五用海盗腔说话", "金曜日は海賊口調で", "Rede freitags wie ein Pirat", "Habla como un pirata los viernes", "По п'ятницях говори як пірат"]),
    ("Write my commit messages like Shakespeare", ["Pisz moje commity jak Szekspir", "用莎士比亚的口吻写提交信息", "コミットメッセージはシェイクスピア風に", "Schreib meine Commit-Nachrichten wie Shakespeare", "Escribe mis commits como Shakespeare", "Пиши мої коміти як Шекспір"]),
    ("Make “per my last email” sound friendly", ["Niech „jak pisałem w poprzednim mailu” brzmi miło", "让“如我上封邮件所说”听起来友好点", "「前回のメールの通り」を優しい言い方に", "Lass „wie in meiner letzten Mail erwähnt“ freundlich klingen", "Haz que «como dije en mi último correo» suene amable", "Зроби «як я писав у попередньому листі» дружнім"]),
    ("Open file", ["Otwórz plik", "打开文件", "ファイルを開く", "Datei öffnen", "Abrir archivo", "Відкрити файл"]),
    ("Your transformations want to use the network", ["Twoje transformacje chcą korzystać z sieci", "你的转换想要使用网络", "変換がネットワークを使おうとしています", "Deine Transformationen möchten das Netzwerk nutzen", "Tus transformaciones quieren usar la red", "Ваші перетворення хочуть використовувати мережу"]),
    ("Your transformations want to run programs", ["Twoje transformacje chcą uruchamiać programy", "你的转换想要运行程序", "変換がプログラムを実行しようとしています", "Deine Transformationen möchten Programme ausführen", "Tus transformaciones quieren ejecutar programas", "Ваші перетворення хочуть запускати програми"]),
    ("Your transformations want to use the network and run programs", ["Twoje transformacje chcą korzystać z sieci i uruchamiać programy", "你的转换想要使用网络并运行程序", "変換がネットワークの使用とプログラムの実行をしようとしています", "Deine Transformationen möchten das Netzwerk nutzen und Programme ausführen", "Tus transformaciones quieren usar la red y ejecutar programas", "Ваші перетворення хочуть використовувати мережу й запускати програми"]),
    ("Hosts: {}", ["Hosty: {}", "主机：{}", "ホスト: {}", "Hosts: {}", "Hosts: {}", "Хости: {}"]),
    ("Programs: {}", ["Programy: {}", "程序：{}", "プログラム: {}", "Programme: {}", "Programas: {}", "Програми: {}"]),
    ("Mumble asks again whenever the config changes. Until you allow it, they paste your words unchanged", ["Mumble zapyta ponownie po każdej zmianie konfiguracji. Dopóki nie zezwolisz, wkleją twoje słowa bez zmian", "配置每次更改后 Mumble 都会再次询问。在你允许之前，它们会原样粘贴你的话", "設定が変わるたびに Mumble は再度確認します。許可するまで、言葉はそのまま貼り付けられます", "Mumble fragt erneut, sobald sich die Konfiguration ändert. Bis du es erlaubst, fügen sie deine Worte unverändert ein", "Mumble vuelve a preguntar cada vez que cambia la configuración. Hasta que lo permitas, pegan tus palabras sin cambios", "Mumble питатиме знову після кожної зміни конфігурації. Доки ви не дозволите, вони вставлятимуть ваші слова без змін"]),
    ("Edit with agent", ["Edytuj z agentem", "用智能体编辑", "エージェントで編集", "Mit Agent bearbeiten", "Editar con agente", "Редагувати з агентом"]),
    ("No steps yet. Mumble pastes your words as it heard them", ["Brak kroków. Mumble wkleja twoje słowa tak, jak je usłyszał", "还没有步骤。Mumble 会按听到的内容粘贴", "まだステップはありません。Mumble は聞き取ったとおりに貼り付けます", "Noch keine Schritte. Mumble fügt deine Worte so ein, wie es sie gehört hat", "Aún no hay pasos. Mumble pega tus palabras tal como las oyó", "Кроків ще немає. Mumble вставляє ваші слова так, як їх почув"]),
    ("Every dictation", ["Każde dyktowanie", "每次听写", "すべての音声入力", "Jedes Diktat", "Cada dictado", "Кожне диктування"]),
    ("Runs top to bottom", ["Od góry do dołu", "从上到下运行", "上から順に実行", "Läuft von oben nach unten", "Se ejecuta de arriba abajo", "Виконується згори донизу"]),
    ("Built in", ["Wbudowany", "内置", "組み込み", "Eingebaut", "Integrado", "Вбудований"]),
    ("Type what you might say", ["Wpisz, co możesz powiedzieć", "输入你可能会说的话", "話しそうなことを入力", "Tippe, was du sagen würdest", "Escribe lo que dirías", "Введіть, що ви могли б сказати"]),
    ("What Mumble pastes", ["Co wklei Mumble", "Mumble 粘贴的内容", "Mumble が貼り付ける内容", "Was Mumble einfügt", "Lo que pega Mumble", "Що вставить Mumble"]),
    ("Try", ["Wypróbuj", "试试", "試す", "Testen", "Probar", "Спробувати"]),
    ("In some apps", ["W niektórych aplikacjach", "在某些应用中", "一部のアプリで", "In manchen Apps", "En algunas apps", "У деяких застосунках"]),
    ("Used there instead", ["Tam zamiast tego", "在那里替代使用", "そこでは代わりに使用", "Dort stattdessen", "Se usan en su lugar", "Там замість цього"]),
    ("Pastes your words as Mumble heard them", ["Wkleja słowa tak, jak Mumble je usłyszał", "按 Mumble 听到的内容粘贴", "Mumble が聞き取ったとおりに貼り付けます", "Fügt deine Worte so ein, wie Mumble sie gehört hat", "Pega tus palabras tal como Mumble las oyó", "Вставляє слова так, як Mumble їх почув"]),
    ("Only: {}", ["Tylko: {}", "仅：{}", "のみ：{}", "Nur: {}", "Solo: {}", "Лише: {}"]),
    ("Copied", ["Skopiowano", "已复制", "コピーしました", "Kopiert", "Copiado", "Скопійовано"]),
    ("Delete", ["Usuń", "删除", "削除", "Löschen", "Eliminar", "Видалити"]),
    ("Remove", ["Usuń", "移除", "削除", "Entfernen", "Quitar", "Вилучити"]),
    ("Dictation", ["Dyktowanie", "听写", "音声入力", "Diktat", "Dictado", "Диктування"]),
    ("Send", ["Wysłano", "发送", "送信", "Senden", "Enviar", "Надсилання"]),
    ("Printed", ["Wypisano", "已输出", "出力", "Ausgegeben", "Impreso", "Виведено"]),
    ("Raw", ["Bez przetwarzania", "原文", "未処理", "Unverarbeitet", "Sin procesar", "Без обробки"]),
    ("Dictate without processing", ["Dyktuj bez przetwarzania", "不经处理的听写", "処理せずに音声入力", "Ohne Verarbeitung diktieren", "Dictar sin procesar", "Диктувати без обробки"]),
    ("Without processing", ["Bez przetwarzania", "不经处理", "処理なし", "Ohne Verarbeitung", "Sin procesar", "Без обробки"]),
    ("Skips every step", ["Pomija wszystkie kroki", "跳过所有步骤", "すべてのステップを省略", "Überspringt jeden Schritt", "Omite todos los pasos", "Пропускає всі кроки"]),
    ("Paste your words as Mumble heard them, skipping every step", ["Wklej słowa tak, jak Mumble je usłyszał, z pominięciem wszystkich kroków", "按 Mumble 听到的内容粘贴，跳过所有步骤", "すべてのステップを省略し、Mumble が聞き取ったとおりに貼り付けます", "Füge deine Worte so ein, wie Mumble sie gehört hat, ohne jeden Schritt", "Pega tus palabras tal como Mumble las oyó, sin ningún paso", "Вставити слова так, як Mumble їх почув, без жодних кроків"]),
    ("Voice Action", ["Voice Action", "Voice Action", "Voice Action", "Voice Action", "Voice Action", "Voice Action"]),
    ("Application", ["Aplikacja", "应用", "アプリ", "Anwendung", "Aplicación", "Програма"]),
    ("Global", ["Globalny", "全局", "グローバル", "Global", "Global", "Глобальний"]),
    ("Fallback for everything", ["Tryb zapasowy dla wszystkiego", "其他所有情况的后备模式", "その他すべてのフォールバック", "Fallback für alles", "Alternativa para todo", "Резервний режим для всього"]),
    ("No activation rules", ["Brak reguł aktywacji", "无激活规则", "有効化ルールなし", "Keine Aktivierungsregeln", "Sin reglas de activación", "Немає правил активації"]),
    ("Untitled mode", ["Tryb bez nazwy", "未命名模式", "名称未設定のモード", "Unbenannter Modus", "Modo sin título", "Режим без назви"]),
    ("Used unless a more specific mode matches.", ["Używany, jeśli nie pasuje bardziej szczegółowy tryb.", "在没有更具体的模式匹配时使用。", "より具体的なモードが一致しない場合に使用します。", "Wird verwendet, wenn kein spezifischerer Modus passt.", "Se usa si no coincide un modo más específico.", "Використовується, якщо не відповідає жоден конкретніший режим."]),
    ("Name", ["Nazwa", "名称", "名前", "Name", "Nombre", "Назва"]),
    ("Applications", ["Aplikacje", "应用", "アプリケーション", "Anwendungen", "Aplicaciones", "Програми"]),
    ("Switch when an app is frontmost", ["Przełącz, gdy aplikacja jest na pierwszym planie", "当前台应用匹配时切换", "アプリが最前面のときに切り替え", "Wechseln, wenn eine App im Vordergrund ist", "Cambiar cuando una aplicación esté en primer plano", "Перемикати, коли програма на передньому плані"]),
    ("Done", ["Gotowe", "完成", "完了", "Fertig", "Listo", "Готово"]),
    ("Corrections", ["Korekty", "更正", "修正", "Korrekturen", "Correcciones", "Виправлення"]),
        ("Microphone", ["Mikrofon", "麦克风", "マイク", "Mikrofon", "Micrófono", "Мікрофон"]),
    ("Dictation shortcut", ["Skrót dyktowania", "听写快捷键", "音声入力ショートカット", "Diktat-Tastenkürzel", "Atajo de dictado", "Комбінація клавіш для диктування"]),
    ("Paste last dictation", ["Wklej ostatnie dyktowanie", "粘贴上次听写", "前回の音声入力を貼り付け", "Letztes Diktat einfügen", "Pegar último dictado", "Вставити останнє диктування"]),
    ("Processing", ["Przetwarzanie", "处理", "処理", "Verarbeitung", "Procesamiento", "Обробка"]),
    ("Shortcut", ["Skrót", "快捷键", "ショートカット", "Kürzel", "Atajo", "Комбінація клавіш"]),
    ("OpenCode", ["OpenCode", "OpenCode", "OpenCode", "OpenCode", "OpenCode", "OpenCode"]),
    ("Default", ["Domyślny", "默认", "既定", "Standard", "Predeterminado", "Типово"]),
    ("Last dictation failed: {}", ["Ostatnie dyktowanie nie powiodło się: {}", "上次听写失败：{}", "前回の音声入力に失敗しました: {}", "Letztes Diktat fehlgeschlagen: {}", "El último dictado falló: {}", "Не вдалося виконати останнє диктування: {}"]),
    ("Version {}", ["Wersja {}", "版本 {}", "バージョン {}", "Version {}", "Versión {}", "Версія {}"]),
    ("Hold", ["Przytrzymaj", "按住", "押しながら", "Halte", "Mantén", "Утримувати"]),
    ("Activity", ["Aktywność", "活动", "アクティビティ", "Aktivität", "Actividad", "Активність"]),
    ("Voice commands", ["Polecenia głosowe", "语音命令", "音声コマンド", "Sprachbefehle", "Comandos de voz", "Голосові команди"]),
    ("While dictating", ["Podczas dyktowania", "听写时", "音声入力中", "Während des Diktats", "Mientras dictas", "Під час диктування"]),
    ("Mute", ["Wycisz", "静音", "ミュート", "Stummschalten", "Silenciar", "Тиша"]),
    ("Pause media", ["Wstrzymaj media", "暂停媒体", "メディアを一時停止", "Medien pausieren", "Pausar medios", "Пауза медіа"]),
    ("Lower audio", ["Ścisz dźwięk", "降低音量", "音量を下げる", "Audio absenken", "Bajar audio", "Приглушити звук"]),
    ("Lower to", ["Ścisz do", "降低至", "音量の割合", "Absenken auf", "Bajar al", "Приглушити до"]),
    ("Percent of the current output volume", ["Procent bieżącej głośności wyjściowej", "当前输出音量的百分比", "現在の出力音量に対する割合", "Prozent der aktuellen Ausgabelautstärke", "Porcentaje del volumen de salida actual", "Відсоток поточної гучності виходу"]),
    ("Volume while recording", ["Głośność podczas nagrywania", "录音时的音量", "録音中の音量", "Lautstärke während der Aufnahme", "Volumen durante la grabación", "Гучність під час запису"]),
    ("Also pause the browser", ["Wstrzymuj też przeglądarkę", "同时暂停浏览器", "ブラウザーも一時停止", "Auch den Browser pausieren", "Pausar también el navegador", "Також призупиняти браузер"]),
    ("YouTube and other tabs, resumed only if Mumble paused them", ["YouTube i inne karty, wznawiane tylko wtedy, gdy wstrzymał je Mumble", "YouTube 和其他标签页，仅在由 Mumble 暂停时才会恢复", "YouTube などのタブ。Mumble が一時停止した場合のみ再開します", "YouTube und andere Tabs, nur fortgesetzt, wenn Mumble sie pausiert hat", "YouTube y otras pestañas; solo se reanudan si Mumble las pausó", "YouTube та інші вкладки; відновлюються, лише якщо їх призупинив Mumble"]),
    ("Lets Mumble hear whether a tab is playing", ["Pozwala Mumble usłyszeć, czy karta odtwarza dźwięk", "让 Mumble 能听出标签页是否在播放", "タブが再生中かどうかを Mumble が聞き取れるようにします", "Damit Mumble hört, ob ein Tab abspielt", "Permite a Mumble oír si una pestaña está reproduciendo", "Дозволяє Mumble чути, чи відтворює вкладка звук"]),
    ("Do nothing", ["Nic nie rób", "不处理", "何もしない", "Nichts tun", "No hacer nada", "Нічого"]),
                ("Launch at login", ["Uruchamiaj przy logowaniu", "登录时启动", "ログイン時に起動", "Beim Anmelden starten", "Iniciar al iniciar sesión", "Запускати під час входу"]),
    ("Top", ["Góra", "顶部", "上", "Oben", "Arriba", "Угорі"]),
    ("Bottom", ["Dół", "底部", "下", "Unten", "Abajo", "Унизу"]),
    ("Off", ["Wył.", "关", "オフ", "Aus", "No", "Вимк."]),
    ("Disabled", ["Wyłączone", "已禁用", "無効", "Deaktiviert", "Desactivado", "Вимкнено"]),
    ("System default", ["Domyślny systemowy", "系统默认", "システムのデフォルト", "Systemstandard", "Predeterminado del sistema", "Системний типовий"]),
    ("Mumble Microphone is the system default, so Mumble listens to {}", ["Mumble Microphone jest domyślny w systemie, więc Mumble słucha przez {}", "Mumble Microphone 是系统默认设备，因此 Mumble 使用 {}", "Mumble Microphone がシステムのデフォルトのため、Mumble は {} を使います", "Mumble Microphone ist der Systemstandard, daher nutzt Mumble {}", "Mumble Microphone es el predeterminado del sistema, así que Mumble usa {}", "Mumble Microphone — системний типовий мікрофон, тож Mumble слухає {}"]),
    ("not connected", ["niepodłączony", "未连接", "未接続", "nicht verbunden", "no conectado", "не під'єднано"]),
    ("{built_in} is off while the lid is closed, so Mumble listens to {input}", ["{built_in} jest wyłączony przy zamkniętej pokrywie, więc Mumble słucha przez {input}", "合盖时 {built_in} 处于关闭状态，因此 Mumble 使用 {input}", "蓋を閉じている間は {built_in} がオフのため、Mumble は {input} を使います", "{built_in} ist bei geschlossenem Deckel aus, daher nutzt Mumble {input}", "{built_in} está apagado con la tapa cerrada, así que Mumble usa {input}", "{built_in} вимкнено, поки кришку закрито, тож Mumble слухає {input}"]),
    ("{} is off while the lid is closed", ["{} jest wyłączony przy zamkniętej pokrywie", "合盖时 {} 处于关闭状态", "蓋を閉じている間は {} がオフです", "{} ist bei geschlossenem Deckel aus", "{} está apagado con la tapa cerrada", "{} вимкнено, поки кришку закрито"]),
    ("off while the lid is closed", ["wyłączony przy zamkniętej pokrywie", "合盖时关闭", "蓋を閉じている間はオフ", "bei geschlossenem Deckel aus", "apagado con la tapa cerrada", "вимкнено, поки кришку закрито"]),
    ("{} isn't connected; using your Mac's input", ["{} nie jest podłączony; używany jest mikrofon Maca", "{} 未连接；正在使用 Mac 的输入设备", "{} は接続されていません。Mac の入力を使用中", "{} ist nicht verbunden; der Eingang des Mac wird verwendet", "{} no está conectado; se usa la entrada del Mac", "{} не під'єднано; використовується вхід вашого Mac"]),
    ("{} isn't connected; using the system input", ["{} nie jest podłączony; używane jest wejście systemowe", "{} 未连接；正在使用系统输入设备", "{} は接続されていません。システムの入力を使用中", "{} ist nicht verbunden; der Systemeingang wird verwendet", "{} no está conectado; se usa la entrada del sistema", "{} не під'єднано; використовується системний вхід"]),
    ("24 hours", ["24 godziny", "24 小时", "24 時間", "24 Stunden", "24 horas", "24 години"]),
    ("7 days", ["7 dni", "7 天", "7 日間", "7 Tage", "7 días", "7 днів"]),
    ("30 days", ["30 dni", "30 天", "30 日間", "30 Tage", "30 días", "30 днів"]),
    ("Forever", ["Bezterminowo", "永久", "無期限", "Unbegrenzt", "Para siempre", "Завжди"]),
    ("No corrections in this mode.", ["Brak korekt w tym trybie.", "此模式中没有更正规则。", "このモードには修正がありません。", "Keine Korrekturen in diesem Modus.", "No hay correcciones en este modo.", "У цьому режимі немає виправлень."]),
    ("No transformations in this mode.", ["Brak transformacji w tym trybie.", "此模式中没有转换。", "このモードには変換がありません。", "Keine Transformationen in diesem Modus.", "No hay transformaciones en este modo.", "У цьому режимі немає перетворень."]),
    ("History is off.", ["Historia jest wyłączona.", "历史记录已关闭。", "履歴はオフです。", "Verlauf ist aus.", "El historial está desactivado.", "Історію вимкнено."]),
    ("Choose how long to keep dictations to see them here.", ["Wybierz, jak długo przechowywać dyktowania, aby widzieć je tutaj.", "选择听写的保留时长，即可在此处查看。", "音声入力を保存する期間を選ぶと、ここに表示されます。", "Wähle, wie lange Diktate behalten werden, um sie hier zu sehen.", "Elige cuánto tiempo guardar los dictados para verlos aquí.", "Виберіть, як довго зберігати диктування, щоб бачити їх тут."]),
    ("Everything you dictate is kept here.", ["Wszystko, co podyktujesz, jest przechowywane tutaj.", "你听写的所有内容都会保存在这里。", "音声入力した内容はすべてここに保存されます。", "Alles, was du diktierst, wird hier aufbewahrt.", "Todo lo que dictes se guarda aquí.", "Усе, що ви диктуєте, зберігається тут."]),
    ("Everything you dictate is kept here for {}.", ["Wszystko, co podyktujesz, jest przechowywane tutaj przez {}.", "你听写的所有内容都会在这里保存 {}。", "音声入力した内容はここに {} 保存されます。", "Alles, was du diktierst, wird hier {} lang aufbewahrt.", "Todo lo que dictes se guarda aquí durante {}.", "Усе, що ви диктуєте, зберігається тут {}."]),
    ("No dictations retained yet.", ["Brak zapisanych dyktowań.", "尚无保存的听写。", "保存された音声入力はまだありません。", "Noch keine Diktate gespeichert.", "Aún no hay dictados guardados.", "Збережених диктувань ще немає."]),
    ("No dictations match.", ["Brak pasujących dyktowań.", "没有匹配的听写。", "一致する音声入力はありません。", "Keine passenden Diktate.", "No hay dictados que coincidan.", "Відповідних диктувань немає."]),
    ("Select a history entry.", ["Wybierz wpis z historii.", "选择一条历史记录。", "履歴の項目を選択してください。", "Wählen Sie einen Verlaufseintrag.", "Selecciona una entrada del historial.", "Виберіть запис з історії."]),
    ("History could not be loaded.", ["Nie udało się wczytać historii.", "无法加载历史记录。", "履歴を読み込めませんでした。", "Verlauf konnte nicht geladen werden.", "No se pudo cargar el historial.", "Не вдалося завантажити історію."]),
    ("Mumble could not apply this change.", ["Mumble nie mógł zastosować tej zmiany.", "Mumble 无法应用此更改。", "Mumble はこの変更を適用できませんでした。", "Mumble konnte diese Änderung nicht übernehmen.", "Mumble no pudo aplicar este cambio.", "Mumble не вдалося застосувати цю зміну."]),
    ("Raw transcript", ["Surowa transkrypcja", "原始转写", "生の文字起こし", "Rohtranskript", "Transcripción sin procesar", "Початковий транскрипт"]),
    ("Ready", ["Gotowy", "就绪", "準備完了", "Bereit", "Listo", "Готово"]),
    ("Listening", ["Nasłuchuje", "正在监听", "リッスン中", "Hört zu", "Escuchando", "Прослуховування"]),
    ("Model required", ["Wymagany model", "需要模型", "モデルが必要", "Modell erforderlich", "Se requiere modelo", "Потрібна модель"]),
    ("Unavailable", ["Niedostępny", "不可用", "利用不可", "Nicht verfügbar", "No disponible", "Недоступно"]),
    ("Recording", ["Nagrywanie", "录音中", "録音中", "Aufnahme", "Grabando", "Запис"]),
    ("Transcribing", ["Transkrypcja", "转写中", "文字起こし中", "Transkribiert", "Transcribiendo", "Розпізнавання"]),
    ("Add", ["Dodaj", "添加", "追加", "Hinzufügen", "Añadir", "Додати"]),
    ("Add Application", ["Dodaj aplikację", "添加应用", "アプリを追加", "App hinzufügen", "Añadir aplicación", "Додати програму"]),
    ("Add a key", ["Dodaj klawisz", "添加一个按键", "キーを追加してください", "Taste hinzufügen", "Añade una tecla", "Додайте клавішу"]),
    ("Add a key Linux can register", ["Dodaj klawisz, który Linux może zarejestrować", "添加一个 Linux 可以注册的按键", "Linux で登録できるキーを追加してください", "Taste hinzufügen, die Linux registrieren kann", "Añade una tecla que Linux pueda registrar", "Додайте клавішу, яку Linux може зареєструвати"]),
    ("Let Mumble hear you", ["Pozwól Mumble cię słyszeć", "让 Mumble 听到你", "Mumble にマイクを使わせてください", "Lass Mumble dich hören", "Deja que Mumble te escuche", "Дозвольте Mumble чути вас"]),
    ("Windows has the microphone turned off for desktop apps.", ["Windows wyłączył mikrofon dla aplikacji klasycznych.", "Windows 已关闭桌面应用的麦克风访问权限。", "Windows でデスクトップ アプリのマイクがオフになっています。", "Windows hat das Mikrofon für Desktop-Apps ausgeschaltet.", "Windows tiene el micrófono desactivado para las aplicaciones de escritorio.", "Windows вимкнув мікрофон для класичних програм."]),
    ("Turn on “Let desktop apps access your microphone” in Windows Settings.", ["Włącz „Zezwalaj aplikacjom klasycznym na dostęp do mikrofonu” w Ustawieniach systemu Windows.", "在 Windows 设置中打开“允许桌面应用访问你的麦克风”。", "Windows の設定で「デスクトップ アプリがマイクにアクセスできるようにする」をオンにしてください。", "Aktiviere „Desktop-Apps den Zugriff auf Ihr Mikrofon erlauben“ in den Windows-Einstellungen.", "Activa «Permitir que las aplicaciones de escritorio accedan al micrófono» en Configuración de Windows.", "Увімкніть «Дозволити класичним програмам доступ до мікрофона» в параметрах Windows."]),
    ("Add a modifier", ["Dodaj modyfikator", "添加一个修饰键", "修飾キーを追加してください", "Sondertaste hinzufügen", "Añade un modificador", "Додайте клавішу-модифікатор"]),
    ("Already in use", ["Już w użyciu", "已被使用", "すでに使用されています", "Bereits belegt", "Ya está en uso", "Уже використовується"]),
    ("Behavior", ["Zachowanie", "行为", "動作", "Verhalten", "Comportamiento", "Поведінка"]),
    ("Change shortcut", ["Zmień skrót", "更改快捷键", "ショートカットを変更", "Kürzel ändern", "Cambiar atajo", "Змінити"]),
    ("Check again", ["Sprawdź ponownie", "再次检查", "再確認", "Erneut prüfen", "Comprobar de nuevo", "Перевірити ще раз"]),
        ("Checking for OpenCode", ["Szukanie OpenCode", "正在查找 OpenCode", "OpenCode を確認中", "Suche nach OpenCode", "Buscando OpenCode", "Перевірка OpenCode"]),
    ("Checking…", ["Sprawdzanie…", "正在检查…", "確認中…", "Wird geprüft…", "Comprobando…", "Перевірка…"]),
    ("Choose how much reasoning the model should use", ["Wybierz, ile rozumowania ma użyć model", "选择模型应使用多少推理", "モデルが使う推論量を選択", "Lege fest, wie viel der Modell nachdenken soll", "Elige cuánto razonamiento debe usar el modelo", "Виберіть рівень міркування моделі"]),
    ("Clear shortcut", ["Wyczyść skrót", "清除快捷键", "ショートカットをクリア", "Kürzel löschen", "Borrar atajo", "Очистити сполучення клавіш"]),
        ("Confirm delete", ["Potwierdź usunięcie", "确认删除", "削除を確定", "Löschen bestätigen", "Confirmar eliminación", "Підтвердити видалення"]),
    ("Create a config file to define reusable transformations.", ["Utwórz plik konfiguracji, aby zdefiniować transformacje wielokrotnego użytku.", "创建一个配置文件来定义可复用的转换。", "設定ファイルを作成して、再利用できる変換を定義します。", "Lege eine Konfigurationsdatei an, um wiederverwendbare Transformationen zu definieren.", "Crea un archivo de configuración para definir transformaciones reutilizables.", "Створіть файл конфігурації, щоб визначити повторно використовувані перетворення."]),
    ("Custom transformations", ["Własne transformacje", "自定义转换", "カスタム変換", "Eigene Transformationen", "Transformaciones personalizadas", "Власні перетворення"]),
    ("Custom transformations are unavailable.", ["Własne transformacje są niedostępne.", "自定义转换不可用。", "カスタム変換を利用できません。", "Eigene Transformationen sind nicht verfügbar.", "Las transformaciones personalizadas no están disponibles.", "Власні перетворення недоступні."]),
    ("Deadline", ["Limit czasu", "期限", "期限", "Zeitlimit", "Plazo", "Ліміт часу"]),
    ("Delete mode", ["Usuń tryb", "删除模式", "モードを削除", "Modus löschen", "Eliminar modo", "Видалити режим"]),
    ("Describe how OpenCode should rewrite your dictation…", ["Opisz, jak OpenCode ma przepisać twoje dyktowanie…", "描述 OpenCode 应如何改写你的听写内容…", "OpenCode が音声入力をどう書き直すか説明してください…", "Beschreibe, wie OpenCode dein Diktat umschreiben soll…", "Describe cómo debe reescribir OpenCode tu dictado…", "Опишіть, як OpenCode має переписувати продиктований текст…"]),
    ("Disable", ["Wyłącz", "停用", "無効にする", "Deaktivieren", "Desactivar", "Вимкнути"]),
    ("Either", ["Dowolny", "任意", "どちらでも", "Beide", "Cualquiera", "Обидві"]),
    ("Enable", ["Włącz", "启用", "有効にする", "Aktivieren", "Activar", "Увімкнути"]),
    ("Enable on launch", ["Włączaj przy uruchomieniu", "启动时启用", "起動時に有効にする", "Beim Start aktivieren", "Activar al iniciar", "Вмикати під час запуску"]),
    ("Every registered transformation is already included.", ["Wszystkie zarejestrowane transformacje są już dodane.", "所有已注册的转换都已包含。", "登録済みの変換はすべて追加されています。", "Alle registrierten Transformationen sind bereits enthalten.", "Todas las transformaciones registradas ya están incluidas.", "Усі зареєстровані перетворення вже додано."]),
    ("Finding applications…", ["Wyszukiwanie aplikacji…", "正在查找应用…", "アプリを検索中…", "Apps werden gesucht…", "Buscando aplicaciones…", "Пошук програм…"]),
    ("Mumble could not load the OpenCode model catalog.", ["Mumble nie mógł wczytać katalogu modeli OpenCode.", "Mumble 无法加载 OpenCode 模型目录。", "Mumble は OpenCode のモデル一覧を読み込めませんでした。", "Mumble konnte den OpenCode-Modellkatalog nicht laden.", "Mumble no pudo cargar el catálogo de modelos de OpenCode.", "Mumble не вдалося завантажити каталог моделей OpenCode."]),
        ("OpenCode isn't running", ["OpenCode nie jest uruchomiony", "OpenCode 未运行", "OpenCode が起動していません", "OpenCode läuft nicht", "OpenCode no se está ejecutando", "OpenCode не запущено"]),
    ("Install or start OpenCode to transform dictated text.", ["Zainstaluj lub uruchom OpenCode, aby przekształcać dyktowany tekst.", "安装或启动 OpenCode 以转换听写文本。", "OpenCode をインストールまたは起動すると、音声入力したテキストを変換できます。", "Installiere oder starte OpenCode, um diktierten Text umzuwandeln.", "Instala o inicia OpenCode para transformar el texto dictado.", "Встановіть або запустіть OpenCode, щоб перетворювати продиктований текст."]),
    ("Instructions", ["Instrukcje", "说明", "指示", "Anweisungen", "Instrucciones", "Інструкції"]),
            ("Latency", ["Opóźnienie", "延迟", "レイテンシー", "Latenz", "Latencia", "Затримка"]),
    ("Left", ["Lewy", "左", "左", "Links", "Izquierda", "Ліва"]),
    ("Loading OpenCode…", ["Wczytywanie OpenCode…", "正在加载 OpenCode…", "OpenCode を読み込み中…", "OpenCode wird geladen…", "Cargando OpenCode…", "Завантаження OpenCode…"]),
    ("Loading custom transformations…", ["Wczytywanie własnych transformacji…", "正在加载自定义转换…", "カスタム変換を読み込み中…", "Eigene Transformationen werden geladen…", "Cargando las transformaciones personalizadas…", "Завантаження власних перетворень…"]),
    ("Mode name", ["Nazwa trybu", "模式名称", "モード名", "Modusname", "Nombre del modo", "Назва режиму"]),
        ("No applications found.", ["Nie znaleziono aplikacji.", "未找到应用。", "アプリが見つかりません。", "Keine Apps gefunden.", "No se encontraron aplicaciones.", "Програм не знайдено."]),
    ("Open OpenCode setup", ["Otwórz konfigurację OpenCode", "打开 OpenCode 设置", "OpenCode のセットアップを開く", "OpenCode-Einrichtung öffnen", "Abrir configuración de OpenCode", "Відкрити налаштування OpenCode"]),
    ("Open settings", ["Otwórz ustawienia", "打开设置", "設定を開く", "Einstellungen öffnen", "Abrir ajustes", "Відкрити налаштування"]),
    ("OpenCode is unavailable", ["OpenCode jest niedostępny", "OpenCode 不可用", "OpenCode を利用できません", "OpenCode ist nicht verfügbar", "OpenCode no está disponible", "OpenCode недоступний"]),
    ("OpenCode model", ["Model OpenCode", "OpenCode 模型", "OpenCode モデル", "OpenCode-Modell", "Modelo de OpenCode", "Модель OpenCode"]),
    ("OpenCode transformation", ["Transformacja OpenCode", "OpenCode 转换", "OpenCode 変換", "OpenCode-Transformation", "Transformación de OpenCode", "Перетворення за допомогою OpenCode"]),
    ("OpenCode transformation will be available after Mumble connects to OpenCode.", ["Transformacja OpenCode będzie dostępna, gdy Mumble połączy się z OpenCode.", "Mumble 连接到 OpenCode 后即可使用 OpenCode 转换。", "Mumble が OpenCode に接続すると OpenCode 変換を使えます。", "Die OpenCode-Transformation ist verfügbar, sobald Mumble mit OpenCode verbunden ist.", "La transformación de OpenCode estará disponible cuando Mumble se conecte a OpenCode.", "Перетворення за допомогою OpenCode стане доступним після підключення Mumble до OpenCode."]),
    ("Share dictation shortcut with other apps", ["Udostępniaj skrót dyktowania innym aplikacjom", "将听写快捷键共享给其他应用", "ディクテーションのショートカットを他のアプリと共有", "Diktierkürzel für andere Apps freigeben", "Compartir el atajo de dictado con otras aplicaciones", "Ділитися сполученням клавіш диктування з іншими програмами"]),
    ("Use another microphone instead of Bluetooth", ["Używaj innego mikrofonu zamiast Bluetooth", "使用其他麦克风而不是蓝牙", "Bluetooth の代わりに別のマイクを使う", "Anderes Mikrofon statt Bluetooth verwenden", "Usar otro micrófono en lugar de Bluetooth", "Використовувати інший мікрофон замість Bluetooth"]),
    ("Use the Mac's microphone instead of Bluetooth", ["Używaj mikrofonu Maca zamiast Bluetooth", "使用 Mac 的麦克风而不是蓝牙", "Bluetooth の代わりに Mac のマイクを使う", "Mikrofon des Mac statt Bluetooth verwenden", "Usar el micrófono del Mac en lugar de Bluetooth", "Використовувати мікрофон Mac замість Bluetooth"]),
    ("Keeps AirPods and other headsets out of call-quality audio", ["Chroni AirPods i inne słuchawki przed jakością dźwięku rozmowy", "避免 AirPods 和其他耳机切换到通话音质", "AirPods などのヘッドセットが通話用の音質にならないようにします", "Hält AirPods und andere Headsets aus der Anrufqualität heraus", "Evita que los AirPods y otros auriculares pasen a calidad de llamada", "Не дає AirPods та іншим гарнітурам перейти на якість дзвінка"]),
    ("{bluetooth} is Bluetooth, so Mumble listens to {input}", ["{bluetooth} to Bluetooth, więc Mumble słucha przez {input}", "{bluetooth} 是蓝牙设备，因此 Mumble 使用 {input}", "{bluetooth} は Bluetooth のため、Mumble は {input} を使います", "{bluetooth} ist Bluetooth, daher nutzt Mumble {input}", "{bluetooth} es Bluetooth, así que Mumble usa {input}", "{bluetooth} — це Bluetooth, тож Mumble слухає {input}"]),
    ("Lets apps such as Discord detect the same key", ["Umożliwia aplikacjom takim jak Discord wykrywanie tego samego klawisza", "允许 Discord 等应用检测同一个按键", "Discord などのアプリが同じキーを検出できるようにします", "Ermöglicht Apps wie Discord, dieselbe Taste zu erkennen", "Permite que aplicaciones como Discord detecten la misma tecla", "Дозволяє програмам на кшталт Discord бачити ту саму клавішу"]),
    ("Paste with Ctrl+Shift+V so terminals accept it", ["Wklejaj przez Ctrl+Shift+V, aby terminale to przyjmowały", "使用 Ctrl+Shift+V 粘贴，以便终端接受", "端末で受け付けられるよう Ctrl+Shift+V で貼り付け", "Mit Strg+Umschalt+V einfügen, damit Terminals es annehmen", "Pegar con Ctrl+Mayús+V para que los terminales lo acepten", "Вставляти за допомогою Ctrl+Shift+V, щоб термінали приймали текст"]),
    ("Permissions needed", ["Wymagane uprawnienia", "需要权限", "権限が必要です", "Berechtigungen erforderlich", "Se necesitan permisos", "Потрібні дозволи"]),
            ("Press shortcut", ["Naciśnij skrót", "按下快捷键", "ショートカットを押す", "Kürzel drücken", "Pulsa el atajo", "Натисніть комбінацію клавіш"]),
    ("Press to start dictating, press again to transcribe", ["Naciśnij, aby zacząć dyktować, i ponownie, aby transkrybować", "按下开始听写，再按一次进行转写", "押すと音声入力を開始し、もう一度押すと文字起こし", "Drücken zum Diktieren, erneut drücken zum Transkribieren", "Pulsa para empezar a dictar y otra vez para transcribir", "Натисніть, щоб почати диктувати, і натисніть знову, щоб розпізнати мовлення"]),
    ("Re-paste the most recent dictation", ["Wklej ponownie ostatnie dyktowanie", "再次粘贴最近一次听写", "最新の音声入力をもう一度貼り付け", "Letztes Diktat erneut einfügen", "Volver a pegar el dictado más reciente", "Повторно вставити останнє диктування"]),
    ("Refresh", ["Odśwież", "刷新", "更新", "Aktualisieren", "Actualizar", "Оновити"]),
    ("Replacement (e.g. OpenCode)", ["Zamiennik (np. OpenCode)", "替换为（例如 OpenCode）", "置換後（例: OpenCode）", "Ersetzung (z. B. OpenCode)", "Reemplazo (p. ej. OpenCode)", "Заміна (наприклад, OpenCode)"]),
    ("Switch to {channel} {version}", ["Przełącz na {channel} {version}", "切换到{channel} {version}", "{channel} {version} に切り替え", "Zu {channel} {version} wechseln", "Cambiar a {channel} {version}", "Перейти на {channel} {version}"]),
    ("Restart to update", ["Uruchom ponownie, aby zaktualizować", "重启以更新", "再起動してアップデート", "Zum Aktualisieren neu starten", "Reiniciar para actualizar", "Перезапустити для оновлення"]),
    ("Retry", ["Ponów", "重试", "再試行", "Erneut versuchen", "Reintentar", "Повторити спробу"]),
    ("Retry setup", ["Ponów konfigurację", "重试设置", "セットアップを再試行", "Einrichtung wiederholen", "Reintentar configuración", "Повторити налаштування"]),
    ("Right", ["Prawy", "右", "右", "Rechts", "Derecha", "Права"]),
    ("Saved", ["Zapisano", "已保存", "保存しました", "Gespeichert", "Guardado", "Збережено"]),
    ("Saving…", ["Zapisywanie…", "正在保存…", "保存中…", "Wird gespeichert…", "Guardando…", "Збереження…"]),
    ("Search applications", ["Szukaj aplikacji", "搜索应用", "アプリを検索", "Apps suchen", "Buscar aplicaciones", "Пошук програм"]),
    ("Search available models", ["Szukaj dostępnych modeli", "搜索可用模型", "利用可能なモデルを検索", "Verfügbare Modelle suchen", "Buscar modelos disponibles", "Пошук доступних моделей"]),
    ("Search history", ["Szukaj w historii", "搜索历史记录", "履歴を検索", "Verlauf durchsuchen", "Buscar en el historial", "Пошук в історії"]),
    ("Seconds", ["Sekundy", "秒", "秒", "Sekunden", "Segundos", "Секунди"]),
    ("Set up", ["Skonfiguruj", "设置", "セットアップ", "Einrichten", "Configurar", "Налаштувати"]),
    ("Shortcut mode", ["Tryb skrótu", "快捷键模式", "ショートカットモード", "Kürzelmodus", "Modo del atajo", "Режим комбінації клавіш"]),
    ("Show Dock icon", ["Pokaż ikonę w Docku", "显示程序坞图标", "Dock にアイコンを表示", "Dock-Symbol anzeigen", "Mostrar icono en el Dock", "Показувати іконку в Dock"]),
    ("Update channel", ["Kanał aktualizacji", "更新渠道", "アップデートチャンネル", "Update-Kanal", "Canal de actualizaciones", "Канал оновлень"]),
    ("Stable", ["Stabilny", "稳定版", "安定版", "Stabil", "Estable", "Стабільний"]),
    ("Dev", ["Deweloperski", "开发版", "開発版", "Entwicklung", "Desarrollo", "Dev"]),
    ("Sound volume", ["Głośność dźwięków", "提示音音量", "効果音の音量", "Lautstärke der Töne", "Volumen de sonidos", "Гучність звуку"]),
    ("Sounds", ["Dźwięki", "提示音", "効果音", "Töne", "Sonidos", "Звуки"]),
    ("Mumble signature", ["Sygnatura Mumble", "Mumble 标志音", "Mumble シグネチャー", "Mumble-Signatur", "Firma de Mumble", "Фірмовий Mumble"]),
    ("Classic", ["Klasyczne", "经典", "クラシック", "Klassisch", "Clásico", "Класичний"]),
    ("Click", ["Klik", "咔嗒", "クリック", "Klick", "Clic", "Клацання"]),
    ("Soft tone", ["Łagodny ton", "柔和音", "ソフトトーン", "Sanfter Ton", "Tono suave", "М’який тон"]),
    ("Wood", ["Drewno", "木质", "ウッド", "Holz", "Madera", "Дерево"]),
    ("Thock", ["Stuk", "键盘闷响", "コトコト", "Thock", "Thock", "Глухий стук"]),
    ("Warm synth", ["Ciepły syntezator", "温暖合成器", "ウォームシンセ", "Warmer Synth", "Sintetizador cálido", "Теплий синтезатор"]),
    ("A kalimba motif that rises and comes home", ["Motyw kalimby, który wznosi się i wraca do domu", "上行后回到主音的卡林巴旋律", "上がって主音に戻るカリンバのモチーフ", "Ein Kalimba-Motiv, das aufsteigt und heimkehrt", "Un motivo de kalimba que sube y vuelve a casa", "Мотив калімби, що здіймається й повертається додому"]),
    ("The tones Mumble first shipped with", ["Dźwięki, z którymi Mumble zadebiutował", "Mumble 最初自带的提示音", "Mumble が最初に搭載していた音", "Die Töne, mit denen Mumble zuerst erschien", "Los tonos con los que Mumble se lanzó", "Звуки, з якими Mumble вийшов уперше"]),
    ("Mechanical key clicks", ["Kliknięcia mechanicznych klawiszy", "机械键盘的咔嗒声", "メカニカルキーのクリック音", "Mechanische Tastenklicks", "Clics de teclas mecánicas", "Клацання механічних клавіш"]),
    ("Warm, rounded blips", ["Ciepłe, zaokrąglone piknięcia", "温暖圆润的短音", "温かく丸みのあるブリップ音", "Warme, runde Blips", "Pitidos cálidos y redondos", "Теплі, округлі сигнали"]),
    ("Soft marimba and hollow wood blocks", ["Miękka marimba i puste drewniane klocki", "柔和的马林巴与空心木鱼", "柔らかなマリンバと中空のウッドブロック", "Sanfte Marimba und hohle Holzblöcke", "Marimba suave y bloques de madera huecos", "М’яка марімба й порожнисті дерев’яні бруски"]),
    ("A deep, creamy keyboard key", ["Głęboki, kremowy klawisz", "低沉圆润的键盘按键", "深くまろやかなキーボードのキー", "Eine tiefe, cremige Tastaturtaste", "Una tecla de teclado grave y cremosa", "Глибока, оксамитова клавіша"]),
    ("A mellow analogue synth", ["Łagodny syntezator analogowy", "柔和的模拟合成器", "まろやかなアナログシンセ", "Ein sanfter Analog-Synth", "Un sintetizador analógico suave", "Лагідний аналоговий синтезатор"]),
    ("Start", ["Start", "开始", "開始", "Start", "Inicio", "Початок"]),
    ("Error", ["Błąd", "错误", "エラー", "Fehler", "Error", "Помилка"]),
    ("Play", ["Odtwórz", "播放", "再生", "Abspielen", "Reproducir", "Відтворити"]),
    ("Sound volume is off", ["Głośność dźwięków jest wyłączona", "提示音音量已关闭", "サウンドの音量はオフです", "Die Tonlautstärke ist aus", "El volumen de los sonidos está desactivado", "Гучність звуків вимкнено"]),
    ("Spoken phrase (e.g. open code)", ["Wypowiadana fraza (np. open code)", "说出的短语（例如 open code）", "話すフレーズ（例: open code）", "Gesprochene Phrase (z. B. open code)", "Frase hablada (p. ej. open code)", "Продиктована фраза (наприклад, open code)"]),
        ("System instructions for rewriting dictation in this mode", ["Instrukcje systemowe do przepisywania dyktowania w tym trybie", "此模式下改写听写内容的系统说明", "このモードで音声入力を書き直すためのシステム指示", "Systemanweisungen zum Umschreiben von Diktaten in diesem Modus", "Instrucciones del sistema para reescribir el dictado en este modo", "Системні інструкції для переписування диктування в цьому режимі"]),
    ("Terminal paste", ["Wklejanie w terminalu", "终端粘贴", "端末への貼り付け", "Einfügen im Terminal", "Pegado en terminal", "Вставлення в термінал"]),
    ("The config could not be created.", ["Nie udało się utworzyć konfiguracji.", "无法创建配置。", "設定を作成できませんでした。", "Die Konfiguration konnte nicht erstellt werden.", "No se pudo crear la configuración.", "Не вдалося створити конфігурацію."]),
    ("Thinking", ["Rozumowanie", "思考", "思考", "Denken", "Razonamiento", "Міркування"]),
    ("Hold or double-tap", ["Przytrzymaj lub stuknij dwa razy", "按住或双击", "長押しまたはダブルタップ", "Halte oder tippe zweimal", "Mantén o pulsa dos veces", "Утримувати або натиснути двічі"]),
    ("Tap", ["Stuknij", "单击", "タップ", "Tippe", "Pulsa", "Натиснути"]),
    ("Double-tap", ["Stuknij dwa razy", "双击", "ダブルタップ", "Tippe zweimal", "Pulsa dos veces", "Натиснути двічі"]),
    ("Hold to talk, or double-tap to talk hands-free", ["Przytrzymaj, aby mówić, lub stuknij dwa razy, aby mówić bez trzymania", "按住说话，或双击免按住说话", "押したまま話すか、ダブルタップして押さずに話します", "Halten zum Sprechen, oder zweimal tippen, um freihändig zu sprechen", "Mantén para hablar, o pulsa dos veces para hablar sin mantener", "Утримуйте, щоб говорити, або натисніть двічі, щоб говорити без рук"]),
    ("Hold to talk, let go to paste", ["Przytrzymaj, aby mówić, puść, aby wkleić", "按住说话，松开粘贴", "押したまま話し、離すと貼り付けます", "Halten zum Sprechen, loslassen zum Einfügen", "Mantén para hablar, suelta para pegar", "Утримуйте, щоб говорити, відпустіть, щоб вставити"]),
    ("Tap to talk hands-free, tap again to paste", ["Stuknij, aby mówić bez trzymania, stuknij ponownie, aby wkleić", "单击免按住说话，再按一次粘贴", "タップして押さずに話し、もう一度タップで貼り付けます", "Tippen, um freihändig zu sprechen, erneut tippen zum Einfügen", "Pulsa para hablar sin mantener, pulsa otra vez para pegar", "Натисніть, щоб говорити без рук, натисніть знову, щоб вставити"]),
    ("Double-tap to talk hands-free, tap again to paste", ["Stuknij dwa razy, aby mówić bez trzymania, stuknij ponownie, aby wkleić", "双击免按住说话，再按一次粘贴", "ダブルタップして押さずに話し、もう一度タップで貼り付けます", "Zweimal tippen, um freihändig zu sprechen, erneut tippen zum Einfügen", "Pulsa dos veces para hablar sin mantener, pulsa otra vez para pegar", "Натисніть двічі, щоб говорити без рук, натисніть знову, щоб вставити"]),
    ("Needs a shortcut with a key, like {}", ["Wymaga skrótu z klawiszem, np. {}", "需要带按键的快捷键，例如 {}", "キーを含むショートカットが必要です（例: {}）", "Braucht ein Kürzel mit einer Taste, etwa {}", "Necesita un atajo con una tecla, como {}", "Потрібна комбінація з клавішею, наприклад {}"]),
    ("Transformations", ["Transformacje", "转换", "変換", "Transformationen", "Transformaciones", "Перетворення"]),
            ("Update", ["Aktualizuj", "更新", "アップデート", "Aktualisieren", "Actualizar", "Оновити"]),
    ("Use OpenCode default", ["Użyj domyślnego OpenCode", "使用 OpenCode 默认值", "OpenCode の既定を使用", "OpenCode-Standard verwenden", "Usar el predeterminado de OpenCode", "Використовувати типову модель OpenCode"]),
    ("Configure mouse shortcuts in your Wayland desktop using the mumble commands", ["Skonfiguruj skróty myszy w ustawieniach pulpitu Wayland za pomocą poleceń mumble", "使用 mumble 命令在 Wayland 桌面设置中配置鼠标快捷键", "mumble コマンドを使って Wayland デスクトップでマウスショートカットを設定してください", "Maus-Shortcuts in den Wayland-Desktop-Einstellungen mit den mumble-Befehlen konfigurieren", "Configura los atajos del ratón en tu escritorio Wayland mediante los comandos mumble", "Налаштуйте сполучення з мишею у своєму робочому столі Wayland за допомогою команд mumble"]),
    ("Wayland doesn't report the frontmost app; these apply under X11", ["Wayland nie podaje aktywnej aplikacji; te reguły działają w X11", "Wayland 不报告前台应用；这些规则仅在 X11 下生效", "Wayland は最前面のアプリを通知しないため、これは X11 でのみ有効です", "Wayland meldet die aktive App nicht; diese Regeln gelten unter X11", "Wayland no informa de la app en primer plano; esto se aplica en X11", "Wayland не повідомляє, яка програма на передньому плані; ці правила діють у X11"]),
    ("Accessibility", ["Dostępność", "辅助功能", "アクセシビリティ", "Bedienungshilfen", "Accesibilidad", "Доступність"]),
    ("Allow", ["Zezwól", "允许", "許可", "Erlauben", "Permitir", "Дозволити"]),
    ("Mumble cannot recognize the dictation shortcut or paste into other apps.", ["Mumble nie rozpozna skrótu dyktowania ani nie wklei tekstu w innych aplikacjach.", "Mumble 无法在其他应用中识别听写快捷键或粘贴文字。", "Mumble はほかのアプリで音声入力ショートカットを認識できず、貼り付けもできません。", "Mumble erkennt das Diktierkürzel in anderen Apps nicht und kann dort nichts einfügen.", "Mumble no puede reconocer el atajo de dictado ni pegar en otras apps.", "Mumble не може розпізнати сполучення клавіш для диктування або вставити текст в інші програми."]),
    ("Mumble cannot record dictation until microphone access is restored.", ["Mumble nie może nagrywać dyktowania, dopóki dostęp do mikrofonu nie zostanie przywrócony.", "在恢复麦克风访问权限之前，Mumble 无法录制听写。", "マイクへのアクセスが戻るまで、Mumble は音声入力を録音できません。", "Mumble kann keine Diktate aufnehmen, bis der Mikrofonzugriff wiederhergestellt ist.", "Mumble no puede grabar dictados hasta que se restablezca el acceso al micrófono.", "Mumble не може записувати диктування, доки доступ до мікрофона не буде відновлено."]),
                        ("Commands", ["Polecenia", "命令", "コマンド", "Befehle", "Comandos", "Команди"]),
            ("Creating config…", ["Tworzenie konfiguracji…", "正在创建配置…", "設定を作成中…", "Konfiguration wird erstellt…", "Creando configuración…", "Створення конфігурації…"]),
                            ("Context", ["Kontekst", "上下文", "コンテキスト", "Kontext", "Contexto", "Контекст"]),
        ("None", ["Brak", "无", "なし", "Keine", "Ninguno", "Немає"]),
    ("Keyboard", ["Klawiatura", "键盘", "キーボード", "Tastatur", "Teclado", "Клавіатура"]),
                        ("Screen & System Audio Recording", ["Nagrywanie ekranu i dźwięku systemu", "屏幕与系统音频录制", "画面とシステムオーディオの収録", "Bildschirm- und Systemaudioaufnahme", "Grabación de pantalla y audio del sistema", "Записування системного звуку й екрана"]),
        ("Starting…", ["Uruchamianie…", "正在启动…", "開始中…", "Wird gestartet…", "Iniciando…", "Запуск…"]),
    ("Open", ["Otwórz", "打开", "開く", "Öffnen", "Abrir", "Відкрити"]),
    ("Failed", ["Niepowodzenie", "失败", "失敗", "Fehlgeschlagen", "Fallido", "Не вдалося"]),
    ("Rename", ["Zmień nazwę", "重命名", "名前を変更", "Umbenennen", "Cambiar nombre", "Перейменувати"]),
    ("Save", ["Zapisz", "保存", "保存", "Speichern", "Guardar", "Зберегти"]),
                                            ("English", ["Angielski", "英语", "英語", "Englisch", "Inglés", "Англійська"]),
    ("Hebrew", ["Hebrajski", "希伯来语", "ヘブライ語", "Hebräisch", "Hebreo", "Іврит"]),
                                                    ("Show Mumble", ["Pokaż Mumble", "显示 Mumble", "Mumble を表示", "Mumble anzeigen", "Mostrar Mumble", "Показати Mumble"]),
    ("Enable / Disable Mumble", ["Włącz / wyłącz Mumble", "启用 / 停用 Mumble", "Mumble を有効 / 無効にする", "Mumble aktivieren / deaktivieren", "Activar / desactivar Mumble", "Увімкнути / вимкнути Mumble"]),
    ("Updated by your package manager", ["Aktualizowany przez menedżer pakietów", "由你的软件包管理器更新", "パッケージマネージャーで更新します", "Wird von deinem Paketmanager aktualisiert", "Se actualiza con tu gestor de paquetes", "Оновлюється через менеджер пакетів"]),
    ("Local", ["Lokalna", "本地", "ローカル", "Lokal", "Local", "Локальна"]),
    ("Test", ["Test", "测试", "テスト", "Test", "Prueba", "Тест"]),
    ("Local build · updates off", ["Lokalna kompilacja · aktualizacje wyłączone", "本地构建 · 更新已关闭", "ローカルビルド · 更新はオフ", "Lokaler Build · Updates aus", "Compilación local · actualizaciones desactivadas", "Локальна збірка · оновлення вимкнено"]),
    ("Updated from your source checkout", ["Aktualizowany z Twojej kopii źródeł", "通过你的源代码检出更新", "ソースのチェックアウトから更新します", "Wird aus deinem Quellcode-Checkout aktualisiert", "Se actualiza desde tu copia del código fuente", "Оновлюється з локальної копії вихідного коду"]),
    ("Quit Mumble", ["Zakończ Mumble", "退出 Mumble", "Mumble を終了", "Mumble beenden", "Salir de Mumble", "Вийти з Mumble"]),
    ("Mumble Dictation", ["Dyktowanie Mumble", "Mumble 听写", "Mumble 音声入力", "Mumble-Diktat", "Dictado de Mumble", "Диктування Mumble"]),
    ("Mumble Dictation: recording", ["Dyktowanie Mumble: nagrywanie", "Mumble 听写：录音中", "Mumble 音声入力: 録音中", "Mumble-Diktat: Aufnahme", "Dictado de Mumble: grabando", "Диктування Mumble: запис"]),
    ("Mumble Dictation: transcribing", ["Dyktowanie Mumble: transkrypcja", "Mumble 听写：转写中", "Mumble 音声入力: 文字起こし中", "Mumble-Diktat: transkribiert", "Dictado de Mumble: transcribiendo", "Диктування Mumble: розпізнавання"]),
    ("Mumble Dictation: update available", ["Dyktowanie Mumble: dostępna aktualizacja", "Mumble 听写：有可用更新", "Mumble 音声入力: アップデートがあります", "Mumble-Diktat: Update verfügbar", "Dictado de Mumble: actualización disponible", "Диктування Mumble: доступне оновлення"]),
    ("Download update", ["Pobierz aktualizację", "下载更新", "アップデートをダウンロード", "Update herunterladen", "Descargar actualización", "Завантажити оновлення"]),
    ("What's new", ["Co nowego", "新功能", "新機能", "Neuigkeiten", "Novedades", "Що нового"]),
    ("Downloading update", ["Pobieranie aktualizacji", "正在下载更新", "アップデートをダウンロード中", "Update wird geladen", "Descargando actualización", "Завантаження оновлення"]),
    ("Managed by your organization", ["Zarządzane przez Twoją organizację", "由你的组织管理", "組織によって管理されています", "Von deiner Organisation verwaltet", "Gestionado por tu organización", "Керується вашою організацією"]),
    ("Enabled", ["Włączony", "已启用", "有効", "Aktiviert", "Activado", "Увімкнено"]),
    ("Sleeping", ["Uśpiony", "休眠中", "スリープ中", "Schläft", "En reposo", "У режимі очікування"]),
    ("Enabling…", ["Włączanie…", "正在启用…", "有効化中…", "Wird aktiviert…", "Activando…", "Увімкнення…"]),
    ("Disabling…", ["Wyłączanie…", "正在停用…", "無効化中…", "Wird deaktiviert…", "Desactivando…", "Вимкнення…"]),
    ("Couldn't enable", ["Nie udało się włączyć", "无法启用", "有効にできませんでした", "Aktivierung fehlgeschlagen", "No se pudo activar", "Не вдалося ввімкнути"]),
        ("Service disconnected", ["Usługa rozłączona", "服务已断开", "サービスとの接続が切れました", "Dienst getrennt", "Servicio desconectado", "Звʼязок зі службою втрачено"]),
    ("Linux can't bind both sides. Use one with a key, like Space", ["Linux nie obsługuje obu stron naraz. Użyj jednej z klawiszem, np. Spacją", "Linux 无法绑定左右两侧。请用其中一侧加一个按键，例如空格", "Linux では左右両方は登録できません。片側とキー（Space など）を使ってください", "Linux kann nicht beide Seiten belegen. Nimm eine mit einer Taste, etwa Leertaste", "Linux no puede usar ambos lados. Usa uno con una tecla, como Espacio", "Linux не прив’язує обидві сторони. Використайте одну з клавішею, як-от Пробіл"]),
    ("Linux needs a key too. Hold these and press one, like Space", ["Linux wymaga też klawisza. Przytrzymaj te i naciśnij jeden, np. Spację", "Linux 还需要一个按键。按住这些键再按一个键，例如空格", "Linux ではキーも必要です。これらを押したままキー（Space など）を押してください", "Linux braucht auch eine Taste. Halte diese und drücke eine, etwa Leertaste", "Linux también necesita una tecla. Mantén estas y pulsa una, como Espacio", "Linux потрібна ще клавіша. Утримуйте ці й натисніть одну, як-от Пробіл"]),
    ("Unsupported key", ["Nieobsługiwany klawisz", "不支持的按键", "未対応のキー", "Nicht unterstützte Taste", "Tecla no compatible", "Непідтримувана клавіша"]),
    ("Choose a file…", ["Wybierz plik…", "选择文件…", "ファイルを選択…", "Datei auswählen…", "Elegir un archivo…", "Вибрати файл…"]),
            ("Enable Mumble to dictate into any app with {}", ["Włącz Mumble, aby dyktować w dowolnej aplikacji za pomocą {}", "启用 Mumble 后，可用 {} 在任意应用中听写", "Mumble を有効にすると、{} でどのアプリにも音声入力できます", "Aktiviere Mumble, um mit {} in jede App zu diktieren", "Activa Mumble para dictar en cualquier app con {}", "Увімкніть Mumble, щоб диктувати в будь-якій програмі за допомогою {}"]),
    ("Ready anywhere · Hold {} to dictate", ["Gotowy wszędzie · Przytrzymaj {}, aby dyktować", "随处可用 · 按住 {} 进行听写", "どこでも使えます · {} を押している間に音声入力", "Überall bereit · {} halten zum Diktieren", "Listo en cualquier lugar · Mantén {} para dictar", "Готово всюди · Утримуйте {}, щоб диктувати"]),
    ("Ready anywhere · Press {} to start and stop dictating", ["Gotowy wszędzie · Naciśnij {}, aby zacząć i zakończyć dyktowanie", "随处可用 · 按 {} 开始和停止听写", "どこでも使えます · {} を押して音声入力を開始・停止", "Überall bereit · {} drücken zum Starten und Beenden des Diktats", "Listo en cualquier lugar · Pulsa {} para empezar y terminar de dictar", "Готово всюди · Натисніть {}, щоб почати й завершити диктування"]),
    ("Keep: {}", ["Przechowuj: {}", "保留：{}", "保持: {}", "Behalten: {}", "Conservar: {}", "Зберігати: {}"]),
    ("No applications match \"{}\".", ["Żadna aplikacja nie pasuje do „{}”.", "没有与“{}”匹配的应用。", "「{}」に一致するアプリはありません。", "Keine App passt zu „{}“.", "Ninguna aplicación coincide con «{}».", "Програм, що відповідають запиту «{}», не знайдено."]),
    ("OpenCode models unavailable: {}", ["Modele OpenCode są niedostępne: {}", "OpenCode 模型不可用：{}", "OpenCode のモデルを利用できません: {}", "OpenCode-Modelle nicht verfügbar: {}", "Modelos de OpenCode no disponibles: {}", "Моделі OpenCode недоступні: {}"]),
    ("{} available models", ["Dostępne modele: {}", "{} 个可用模型", "利用可能なモデル {} 件", "{} verfügbare Modelle", "{} modelos disponibles", "Доступні моделі: {}"]),
    ("Mode {}", ["Tryb {}", "模式 {}", "モード {}", "Modus {}", "Modo {}", "Режим {}"]),
    ("{name} already activates {conflict}. Remove it there before reassigning it.", ["{name} już aktywuje tryb {conflict}. Usuń go tam przed ponownym przypisaniem.", "{name} 已用于激活 {conflict}。请先从那里移除再重新分配。", "{name} はすでに {conflict} を有効にしています。割り当て直す前にそちらから削除してください。", "{name} aktiviert bereits {conflict}. Entferne es dort, bevor du es neu zuweist.", "{name} ya activa {conflict}. Quítalo de allí antes de reasignarlo.", "{name} уже активує режим {conflict}. Вилучіть його звідти, перш ніж призначати повторно."]),
    ("Could not restart Mumble: {}", ["Nie udało się ponownie uruchomić Mumble: {}", "无法重启 Mumble：{}", "Mumble を再起動できませんでした: {}", "Mumble konnte nicht neu gestartet werden: {}", "No se pudo reiniciar Mumble: {}", "Не вдалося перезапустити Mumble: {}"]),
    ("Could not start listener: {}", ["Nie udało się uruchomić nasłuchiwania: {}", "无法启动监听：{}", "リスナーを開始できませんでした: {}", "Listener konnte nicht gestartet werden: {}", "No se pudo iniciar la escucha: {}", "Не вдалося запустити службу прослуховування: {}"]),
    ("{audio} ms audio · {inference} ms inference", ["{audio} ms dźwięku · {inference} ms wnioskowania", "音频 {audio} 毫秒 · 推理 {inference} 毫秒", "音声 {audio} ms · 推論 {inference} ms", "{audio} ms Audio · {inference} ms Inferenz", "{audio} ms de audio · {inference} ms de inferencia", "{audio} мс аудіо · {inference} мс обробки"]),
    ("{} ms total", ["łącznie {} ms", "总计 {} 毫秒", "合計 {} ms", "{} ms gesamt", "{} ms en total", "Усього: {} мс"]),
    ("fell back: {}", ["użyto oryginału: {}", "已回退为原文：{}", "元のテキストを使用: {}", "Rohtext verwendet: {}", "se usó el texto original: {}", "використано оригінал: {}"]),
        ("Device Control and Data Access", ["Sterowanie urządzeniem i dostęp do danych", "设备控制与数据访问", "デバイス制御とデータアクセス", "Gerätesteuerung und Datenzugriff", "Control del dispositivo y acceso a datos", "Керування пристроями й доступ до даних"]),
    ("Allowed", ["Dozwolone", "已允许", "許可済み", "Erlaubt", "Permitido", "Дозволено"]),
    ("Back", ["Wstecz", "返回", "戻る", "Zurück", "Atrás", "Назад"]),
    ("Continue", ["Dalej", "继续", "続ける", "Weiter", "Continuar", "Продовжити"]),
    ("Almost ready", ["Prawie gotowe", "即将就绪", "もうすぐ準備完了", "Fast bereit", "Casi listo", "Майже готово"]),
    ("Let Mumble paste for you", ["Pozwól Mumble wklejać za ciebie", "让 Mumble 替你粘贴", "Mumble に貼り付けを任せる", "Lass Mumble für dich einfügen", "Deja que Mumble pegue por ti", "Дозвольте Mumble вставляти за вас"]),
    ("Mumble needs this to type your words into your apps. Your desktop asks once, and may call it remote desktop.", ["Mumble potrzebuje tego, by wpisywać twoje słowa w aplikacjach. Pulpit zapyta raz i może nazwać to zdalnym pulpitem.", "Mumble 需要此权限才能把你的话输入到应用中。桌面只会询问一次，可能会称之为“远程桌面”。", "Mumble がアプリに言葉を入力するために必要です。デスクトップが一度だけ確認し、「リモートデスクトップ」と表示することがあります。", "Mumble braucht das, um deine Worte in deine Apps zu schreiben. Dein Desktop fragt einmal und nennt es vielleicht Fernsteuerung.", "Mumble lo necesita para escribir tus palabras en tus apps. Tu escritorio lo pregunta una vez y puede llamarlo escritorio remoto.", "Mumble потрібен цей дозвіл, щоб вводити ваші слова в застосунки. Стільниця запитає один раз і може назвати це віддаленою стільницею."]),
    ("Paste into apps", ["Wklejanie w aplikacjach", "粘贴到应用", "アプリへの貼り付け", "In Apps einfügen", "Pegar en apps", "Вставлення в застосунки"]),
    ("Mumble only sends the paste keystroke. It never sees your screen.", ["Mumble wysyła tylko skrót wklejania. Nigdy nie widzi twojego ekranu.", "Mumble 只发送粘贴按键，从不查看你的屏幕。", "Mumble は貼り付けのキー操作を送るだけで、画面を見ることはありません。", "Mumble sendet nur den Einfüge-Tastendruck. Deinen Bildschirm sieht es nie.", "Mumble solo envía la tecla de pegar. Nunca ve tu pantalla.", "Mumble лише надсилає комбінацію вставлення. Він ніколи не бачить ваш екран."]),
    ("Without it, Mumble copies your words and you press Ctrl+V.", ["Bez tego Mumble kopiuje twoje słowa, a ty naciskasz Ctrl+V.", "没有此权限时，Mumble 会复制你的话，由你按 Ctrl+V。", "許可しない場合、Mumble が言葉をコピーし、あなたが Ctrl+V を押します。", "Ohne das kopiert Mumble deine Worte und du drückst Strg+V.", "Sin esto, Mumble copia tus palabras y tú pulsas Ctrl+V.", "Без цього Mumble копіює ваші слова, а ви натискаєте Ctrl+V."]),
    ("Hears you only while you hold the shortcut", ["Słyszy cię tylko, gdy trzymasz skrót", "仅在按住快捷键时收听", "ショートカットを押している間だけ聞き取ります", "Hört nur zu, während du das Kürzel hältst", "Solo te escucha mientras mantienes el atajo", "Слухає вас, лише поки ви утримуєте комбінацію клавіш"]),
    ("Let Mumble hear you and type for you", ["Pozwól Mumble słyszeć cię i pisać za ciebie", "让 Mumble 听见你并替你输入", "Mumble が聞き取り、入力できるようにする", "Lass Mumble zuhören und für dich tippen", "Deja que Mumble te escuche y escriba por ti", "Дозвольте Mumble слухати вас і друкувати за вас"]),
    ("Notices the shortcut and pastes your words in any app", ["Rozpoznaje skrót i wkleja słowa w każdej aplikacji", "在所有应用中识别快捷键并粘贴文字", "どのアプリでもショートカットを認識し、言葉を貼り付けます", "Erkennt das Kürzel und fügt deine Worte in jeder App ein", "Detecta el atajo y pega tus palabras en cualquier app", "Помічає сполучення клавіш і вставляє ваші слова в будь-якій програмі"]),
    ("Quit and reopen Mumble to finish setup. macOS applies some permissions only after a restart.", ["Zamknij i ponownie otwórz Mumble, aby dokończyć konfigurację. macOS stosuje niektóre uprawnienia dopiero po ponownym uruchomieniu.", "请退出并重新打开 Mumble 以完成设置。macOS 的部分权限需要重启后才会生效。", "設定を完了するには Mumble を終了して開き直してください。macOS の一部の許可は再起動後に有効になります。", "Beende Mumble und öffne es erneut, um die Einrichtung abzuschließen. macOS übernimmt manche Berechtigungen erst nach einem Neustart.", "Cierra y vuelve a abrir Mumble para terminar la configuración. macOS aplica algunos permisos solo tras reiniciar.", "Завершіть роботу Mumble і відкрийте його знову, щоб завершити налаштування. macOS застосовує деякі дозволи лише після перезапуску."]),
    ("Reopen Mumble", ["Otwórz Mumble ponownie", "重新打开 Mumble", "Mumble を開き直す", "Mumble neu öffnen", "Reabrir Mumble", "Відкрити Mumble знову"]),
    ("Restart Mumble", ["Uruchom Mumble ponownie", "重新启动 Mumble", "Mumble を再起動", "Mumble neu starten", "Reiniciar Mumble", "Перезапустити Mumble"]),
    ("Turns on when Mumble restarts", ["Włączy się po ponownym uruchomieniu Mumble", "Mumble 重新启动后生效", "Mumble の再起動後に有効になります", "Wird nach einem Neustart von Mumble aktiv", "Se activa cuando Mumble se reinicia", "Увімкнеться після перезапуску Mumble"]),
    ("Skip", ["Pomiń", "跳过", "スキップ", "Überspringen", "Omitir", "Пропустити"]),
                            ("Language", ["Język", "语言", "言語", "Sprache", "Idioma", "Мова"]),
                                ("Languages", ["Języki", "语言", "言語", "Sprachen", "Idiomas", "Мови"]),
            ("Yes", ["Tak", "是", "はい", "Ja", "Sí", "Так"]),
            ("{} s", ["{} s", "{} 秒", "{} 秒", "{} s", "{} s", "{} с"]),
                                    ("of {}", ["z {}", "共 {}", "/ {}", "von {}", "de {}", "з {}"]),
    ("In use", ["W użyciu", "使用中", "使用中", "In Verwendung", "En uso", "Використовується"]),
                                                                                                        ("Start using Mumble", ["Zacznij używać Mumble", "开始使用 Mumble", "Mumble を使い始める", "Mumble verwenden", "Empezar a usar Mumble", "Почати користуватися Mumble"]),
    ("Transcribing…", ["Transkrybowanie…", "正在转写…", "文字起こし中…", "Transkribiert …", "Transcribiendo…", "Розпізнавання мовлення…"]),
    // Fun / Mumble Microphone (experimental)
    ("Available", ["Dostępny", "可用", "利用可能", "Verfügbar", "Disponible", "Доступний"]),
    ("Virtual microphone", ["Wirtualny mikrofon", "虚拟麦克风", "仮想マイク", "Virtuelles Mikrofon", "Micrófono virtual", "Віртуальний мікрофон"]),
    ("Could not start Mumble Microphone: {}", ["Nie udało się uruchomić Mumble Microphone: {}", "无法启动 Mumble Microphone：{}", "Mumble Microphone を開始できませんでした: {}", "Mumble Microphone konnte nicht gestartet werden: {}", "No se pudo iniciar Mumble Microphone: {}", "Не вдалося запустити Mumble Microphone: {}"]),
    ("Starting Mumble Microphone…", ["Uruchamianie Mumble Microphone…", "正在启动 Mumble Microphone…", "Mumble Microphone を開始中…", "Mumble Microphone wird gestartet…", "Iniciando Mumble Microphone…", "Запуск Mumble Microphone…"]),
    ("Call hears", ["Rozmówcy słyszą", "通话对方听到", "通話相手に聞こえる音", "Anruf hört", "La llamada oye", "Що чує співрозмовник"]),
    ("Carrier", ["Fala nośna", "载波频率", "キャリア周波数", "Trägerfrequenz", "Portadora", "Несуча частота"]),
    ("Chipmunk", ["Wiewiórka", "花栗鼠", "チップマンク", "Chipmunk", "Ardilla", "Бурундук"]),
    ("Choose Mumble Microphone as your input in Slack, Zoom, or Discord", ["Wybierz Mumble Microphone jako wejście w Slacku, Zoomie lub Discordzie", "在 Slack、Zoom 或 Discord 中选择 Mumble Microphone 作为输入设备", "Slack、Zoom、または Discord の入力として Mumble Microphone を選択してください", "Wähle Mumble Microphone als Eingabegerät in Slack, Zoom oder Discord", "Elige Mumble Microphone como entrada en Slack, Zoom o Discord", "Виберіть Mumble Microphone як пристрій введення в Slack, Zoom або Discord"]),
    ("Comfort noise", ["Szum tła", "舒适背景音", "コンフォートノイズ", "Komfortrauschen", "Ruido de confort", "Комфортний шум"]),
    ("Compare your raw microphone with what the call hears during dictation", ["Porównaj surowy dźwięk mikrofonu z tym, co słyszą rozmówcy podczas dyktowania", "对比原始麦克风声音与听写时通话对方听到的声音", "元のマイク音声とディクテーション中に相手へ届く音を比較します", "Vergleiche dein Originalmikrofon mit dem, was der Anruf beim Diktieren hört", "Compara tu micrófono original con lo que oye la llamada mientras dictas", "Порівняйте необроблений звук із мікрофона з тим, що чують співрозмовники під час диктування"]),
    ("Crunch", ["Przester", "失真度", "クランチ", "Crunch", "Saturación", "Спотворення"]),
    ("Depth", ["Głębokość", "深度", "深さ", "Tiefe", "Profundidad", "Глибина"]),
    ("Driver not installed", ["Sterownik niezainstalowany", "驱动未安装", "ドライバ未インストール", "Treiber nicht installiert", "Controlador no instalado", "Драйвер не встановлено"]),
    ("Effect strength", ["Siła efektu", "效果强度", "エフェクトの強さ", "Effektstärke", "Intensidad del efecto", "Інтенсивність ефекту"]),
    ("Example", ["Przykład", "示例", "サンプル", "Beispiel", "Ejemplo", "Приклад"]),
        ("Far", ["Z daleka", "远处", "遠くから", "Weit weg", "Lejos", "Здалеку"]),
    ("Mumble Microphone", ["Mumble Microphone", "Mumble Microphone", "Mumble Microphone", "Mumble Microphone", "Mumble Microphone", "Mumble Microphone"]),
    ("In use by {}", ["Używany przez: {}", "{} 正在使用", "{} が使用中", "Verwendet von {}", "En uso por {}", "Використовується: {}"]),
    ("Install", ["Zainstaluj", "安装", "インストール", "Installieren", "Instalar", "Встановити"]),
    ("Install the CoreAudio driver to choose Mumble Microphone in Slack, Zoom, or Discord", ["Zainstaluj sterownik CoreAudio, aby wybrać Mumble Microphone w Slacku, Zoomie lub Discordzie", "安装 CoreAudio 驱动后即可在 Slack、Zoom 或 Discord 中选择 Mumble Microphone", "CoreAudio ドライバをインストールすると、Slack・Zoom・Discord で Mumble Microphone を選べます", "Installiere den CoreAudio-Treiber, um Mumble Microphone in Slack, Zoom oder Discord auszuwählen", "Instala el controlador CoreAudio para elegir Mumble Microphone en Slack, Zoom o Discord", "Встановіть драйвер CoreAudio, щоб вибрати Mumble Microphone у Slack, Zoom або Discord"]),
    ("Installation cancelled", ["Instalacja anulowana", "已取消安装", "インストールがキャンセルされました", "Installation abgebrochen", "Instalación cancelada", "Встановлення скасовано"]),
    ("Installed v{}", ["Zainstalowano v{}", "已安装 v{}", "v{} インストール済み", "v{} installiert", "Instalado v{}", "Встановлено v{}"]),
    ("Installing driver and restarting Core Audio…", ["Instalowanie sterownika i restartowanie Core Audio…", "正在安装驱动并重启 Core Audio…", "ドライバをインストールし Core Audio を再起動中…", "Treiber wird installiert und Core Audio neu gestartet…", "Instalando controlador y reiniciando Core Audio…", "Встановлення драйвера й перезапуск Core Audio…"]),
    ("Installing…", ["Instalowanie…", "正在安装…", "インストール中…", "Wird installiert…", "Instalando…", "Встановлення…"]),
    ("Cancelled", ["Anulowano", "已取消", "キャンセル", "Abgebrochen", "Cancelado", "Скасовано"]),
    ("Could not install driver: {}", ["Nie udało się zainstalować sterownika: {}", "无法安装驱动：{}", "ドライバをインストールできませんでした: {}", "Treiber konnte nicht installiert werden: {}", "No se pudo instalar el controlador: {}", "Не вдалося встановити драйвер: {}"]),
    ("Could not update driver: {}", ["Nie udało się zaktualizować sterownika: {}", "无法更新驱动：{}", "ドライバを更新できませんでした: {}", "Treiber konnte nicht aktualisiert werden: {}", "No se pudo actualizar el controlador: {}", "Не вдалося оновити драйвер: {}"]),
    ("Driver removed", ["Sterownik usunięty", "驱动已移除", "ドライバを削除しました", "Treiber entfernt", "Controlador eliminado", "Драйвер видалено"]),
    ("Removed", ["Usunięto", "已移除", "削除済み", "Entfernt", "Eliminado", "Видалено"]),
    ("Removing driver and restarting Core Audio…", ["Usuwanie sterownika i restartowanie Core Audio…", "正在移除驱动并重启 Core Audio…", "ドライバを削除し Core Audio を再起動中…", "Treiber wird entfernt und Core Audio neu gestartet…", "Eliminando controlador y reiniciando Core Audio…", "Видалення драйвера й перезапуск Core Audio…"]),
    ("Uninstall", ["Odinstaluj", "卸载", "アンインストール", "Deinstallieren", "Desinstalar", "Видалити"]),
    ("Uninstall cancelled", ["Odinstalowanie anulowane", "已取消卸载", "アンインストールがキャンセルされました", "Deinstallation abgebrochen", "Desinstalación cancelada", "Видалення скасовано"]),
    ("Uninstalling…", ["Odinstalowywanie…", "正在卸载…", "アンインストール中…", "Wird deinstalliert…", "Desinstalando…", "Видалення…"]),
    ("Update cancelled", ["Aktualizacja anulowana", "已取消更新", "更新がキャンセルされました", "Update abgebrochen", "Actualización cancelada", "Оновлення скасовано"]),
    ("Update driver to v{}", ["Zaktualizuj sterownik do v{}", "将驱动更新到 v{}", "ドライバを v{} に更新", "Treiber auf v{} aktualisieren", "Actualizar controlador a v{}", "Оновити драйвер до v{}"]),
    ("Update v{}", ["Aktualizacja v{}", "更新 v{}", "v{} に更新", "Update v{}", "Actualizar v{}", "Оновлення v{}"]),
    ("v{} installed", ["Zainstalowano v{}", "已安装 v{}", "v{} インストール済み", "v{} installiert", "v{} instalado", "v{} встановлено"]),
    ("Keeps a quiet room tone while muted so callers know the line is live", ["Zachowuje cichy szum pokoju podczas wyciszenia, by rozmówcy wiedzieli, że połączenie trwa", "静音时保留轻微环境底噪，让对方知道通话仍在线", "ミュート中もわずかなルームトーンを残し、通話が繋がっていることを伝えます", "Behält beim Stummschalten einen leisen Raumton bei, damit die Leitung lebendig wirkt", "Mantiene un tono ambiental suave al silenciar para que la llamada siga viva", "Підтримує тихий фоновий шум, коли мікрофон вимкнено, щоб співрозмовники знали, що звʼязок активний"]),
    ("Level", ["Poziom", "音量", "レベル", "Pegel", "Nivel", "Рівень"]),
    ("Mode while dictating", ["Tryb podczas dyktowania", "听写时的声音模式", "ディクテーション中のモード", "Modus beim Diktieren", "Modo al dictar", "Режим під час диктування"]),
    ("Muffled", ["Przytłumiony", "捂麦", "こもった声", "Gedämpft", "Amortiguado", "Приглушений"]),
    ("Muted", ["Wyciszony", "静音", "ミュート", "Stumm", "Silenciado", "Без звуку"]),
    ("Normal", ["Normalny", "正常", "通常", "Normal", "Normal", "Звичайний"]),
        ("Passes your voice through unchanged while you dictate", ["Przekazuje twój głos bez zmian podczas dyktowania", "听写时原样传递你的声音", "ディクテーション中も声をそのまま通します", "Leitet deine Stimme beim Diktieren unverändert weiter", "Deja pasar tu voz sin cambios mientras dictas", "Передає ваш голос без змін під час диктування"]),
    ("Pitch", ["Wysokość tonu", "音高", "ピッチ", "Tonhöhe", "Tono", "Висота тону"]),
    ("Play what call hears", ["Odtwórz co słyszy rozmówca", "播放通话对方听到的声音", "相手に聞こえる音を再生", "Abspielen, was der Anruf hört", "Reproducir lo que oye la llamada", "Відтворити те, що чує співрозмовник"]),
    ("Play your voice", ["Odtwórz swój głos", "播放你的原声", "自分の声を再生", "Eigene Stimme abspielen", "Reproducir tu voz", "Відтворити ваш голос"]),
    ("Quiet", ["Cichy", "轻声", "小声", "Leise", "Suave", "Тихий"]),
    ("Record 5s", ["Nagraj 5 s", "录制 5 秒", "5秒録音", "5 Sek. aufnehmen", "Grabar 5 s", "Записати 5 с"]),
    ("Reset", ["Resetuj", "重置", "リセット", "Zurücksetzen", "Restablecer", "Скинути"]),
    ("Robot", ["Robot", "机器人", "ロボット", "Roboter", "Robot", "Робот"]),
    ("Room", ["Pogłos", "房间混响", "ルーム残響", "Raumklang", "Sala", "Кімната"]),
    ("Room tone", ["Szum pokoju", "环境底噪", "ルームトーン", "Raumton", "Tono ambiental", "Фоновий шум приміщення"]),
    ("Silences your voice with soft room tone while you dictate", ["Wycisza twój głos delikatnym szumem pokoju podczas dyktowania", "听写时用柔和的环境底噪隐去你的声音", "ディクテーション中は柔らかなルームトーンで声をミュートします", "Schaltet deine Stimme beim Diktieren mit sanftem Raumton stumm", "Silencia tu voz con un suave tono ambiental mientras dictas", "Заглушує ваш голос, додаючи тихий фоновий шум приміщення під час диктування"]),
    ("Sounds like a hand over the microphone while you dictate", ["Brzmi jak dłoń zakrywająca mikrofon podczas dyktowania", "听写时听起来像用手捂住了麦克风", "ディクテーション中、マイクを手で覆ったような音になります", "Klingt beim Diktieren wie eine Hand über dem Mikrofon", "Suena como si taparas el micrófono con la mano mientras dictas", "Звучить так, ніби під час диктування мікрофон прикрито рукою"]),
    ("Sounds like a handheld radio while you dictate", ["Brzmi jak krótkofalówka podczas dyktowania", "听写时听起来像对讲机", "ディクテーション中、トランシーバーのような音になります", "Klingt beim Diktieren wie ein Funkgerät", "Suena como un walkie-talkie mientras dictas", "Звучить як рація під час диктування"]),
    ("Sounds like you are across the room while you dictate", ["Brzmi jakbyś mówił z drugiego końca pokoju podczas dyktowania", "听写时听起来像在房间另一头说话", "ディクテーション中、部屋の離れた場所から話しているように聞こえます", "Klingt beim Diktieren, als wärst du am anderen Ende des Raums", "Suena como si hablaras desde el otro lado de la habitación mientras dictas", "Звучить так, ніби ви говорите з іншого кінця кімнати під час диктування"]),
    ("Sounds pitched up while you dictate", ["Podwyższa ton głosu podczas dyktowania", "听写时声音变尖", "ディクテーション中、声が高くなります", "Klingt beim Diktieren höher gepitcht", "Suena con tono agudo mientras dictas", "Звучить вище за тоном під час диктування"]),
    ("Sounds robotic while you dictate", ["Brzmi jak głos robota podczas dyktowania", "听写时听起来像机器人", "ディクテーション中、ロボットのような声になります", "Klingt beim Diktieren wie ein Roboter", "Suena robótico mientras dictas", "Звучить як голос робота під час диктування"]),
    ("Sounds submerged while you dictate", ["Brzmi jak spod wody podczas dyktowania", "听写时听起来像在水下", "ディクテーション中、水中にいるような音になります", "Klingt beim Diktieren wie unter Wasser", "Suena como bajo el agua mientras dictas", "Звучить так, ніби ви під водою, під час диктування"]),
    ("Stays in memory", ["Tylko w pamięci", "仅保存在内存中", "メモリ内のみに保持", "Bleibt im Speicher", "Solo en memoria", "Залишається в памʼяті"]),
    ("Stop", ["Zatrzymaj", "停止", "停止", "Stopp", "Detener", "Зупинити"]),
    ("Stop recording", ["Zatrzymaj nagrywanie", "停止录制", "録音を停止", "Aufnahme stoppen", "Detener grabación", "Зупинити запис"]),
    ("Streaming 48 kHz audio to {}", ["Strumieniowanie 48 kHz do: {}", "正在向 {} 传输 48 kHz 音频", "{} へ 48 kHz 音声を配信中", "Streamt 48-kHz-Audio an {}", "Transmitiendo audio de 48 kHz a {}", "Передавання аудіо 48 кГц до {}"]),
        ("Try it", ["Wypróbuj", "试听与录制", "試してみる", "Ausprobieren", "Pruébalo", "Спробувати"]),
    ("Turns your voice down to a whisper while you dictate", ["Ścisza twój głos do szeptu podczas dyktowania", "听写时将你的声音压低到耳语音量", "ディクテーション中、声をささやき声まで下げます", "Senkt deine Stimme beim Diktieren auf Flüstern ab", "Baja tu voz a un susurro mientras dictas", "Знижує гучність вашого голосу до шепоту під час диктування"]),
    ("Underwater", ["Pod wodą", "水下", "水中", "Unter Wasser", "Bajo el agua", "Під водою"]),
    ("Virtual microphone driver", ["Sterownik wirtualnego mikrofonu", "虚拟麦克风驱动", "仮想マイクドライバ", "Virtueller Mikrofontreiber", "Controlador de micrófono virtual", "Драйвер віртуального мікрофона"]),
    ("Walkie", ["Krótkofalówka", "对讲机", "トランシーバー", "Walkie-Talkie", "Walkie", "Рація"]),
    ("Wall", ["Tłumienie", "闷音滤波", "こもり具合", "Dämpfung", "Filtro", "Приглушення"]),
    ("You", ["Ty", "你", "あなた", "Du", "Tú", "Ви"]),
    ("Your 5s sample", ["Twoja próbka 5 s", "你的 5 秒录音", "あなたの5秒サンプル", "Deine 5-Sek.-Aufnahme", "Tu muestra de 5 s", "Ваш зразок на 5 с"]),
    ("Your voice", ["Twój głos", "你的声音", "自分の声", "Deine Stimme", "Tu voz", "Ваш голос"]),
    // The OS-only Settings rows.
    ("Still grey? Fix it", ["Nadal szare? Napraw", "还是灰色的？修复", "まだグレーのまま？修復", "Noch grau? Beheben", "¿Sigue gris? Reparar", "Ще сіре? Виправити"]),
    ("An older Mumble entry can keep this off", ["Starszy wpis Mumble może to blokować", "旧版 Mumble 条目可能会导致此项无法开启", "古い Mumble の項目が妨げになっている場合があります", "Ein älterer Mumble-Eintrag kann dies blockieren", "Una entrada anterior de Mumble puede impedirlo", "Старий запис Mumble може блокувати це"]),
    ("and answer the one that's lit", ["i odpowiedz na podświetloną", "并回复亮起的那条", "して、光っているメッセージに答えます", "und beantworte die hervorgehobene", "y responde al que está iluminado", "і дайте відповідь на підсвічене"]),
    ("and say anything.", ["i powiedz cokolwiek.", "，随便说点什么。", "して、何か話してください。", "und sag irgendetwas.", "y di cualquier cosa.", "і скажіть будь-що."]),
    ("and say it.", ["i to powiedz.", "，说出来。", "して、話してください。", "und sprich es aus.", "y dilo.", "і скажіть це."]),
    ("Answering under pressure, the usual way.", ["Odpowiadanie pod presją, po staremu.", "在压力下回复，用老办法。", "急かされながら、いつもの方法で返信。", "Antworten unter Druck, wie immer.", "Respondiendo bajo presión, como siempre.", "Відповідаєте під тиском, як зазвичай."]),
    ("anywhere, then talk.", ["gdziekolwiek i mów.", "，在任何地方开口说。", "どこでも、そのまま話します。", "irgendwo und sprich.", "en cualquier lugar y habla.", "будь-де й говоріть."]),
    ("by voice, no typing", ["głosem, bez pisania", "用语音，无需打字", "声だけで、タイピングなし", "per Stimme, ohne Tippen", "por voz, sin teclear", "голосом, без набору"]),
    ("Certificate of completion", ["Certyfikat ukończenia", "完成证书", "修了証", "Abschlusszertifikat", "Certificado de finalización", "Сертифікат про завершення"]),
    ("Clear your messages", ["Wyczyść wiadomości", "清空你的消息", "メッセージを片付けましょう", "Räum deine Nachrichten ab", "Despeja tus mensajes", "Розберіть свої повідомлення"]),
    ("destroyed", ["zniszczone", "已消灭", "撃破", "erledigt", "destruidos", "знищено"]),
    ("fast", ["szybko", "很快", "速い", "schnell", "rápido", "швидко"]),
            ("Five times faster", ["Pięć razy szybciej", "快五倍", "5 倍速い", "Fünfmal schneller", "Cinco veces más rápido", "У п'ять разів швидше"]),
    ("Four times faster", ["Cztery razy szybciej", "快四倍", "4 倍速い", "Viermal schneller", "Cuatro veces más rápido", "Учетверо швидше"]),
    ("Three times faster", ["Trzy razy szybciej", "快三倍", "3 倍速い", "Dreimal schneller", "Tres veces más rápido", "Утричі швидше"]),
    ("Twice as fast", ["Dwa razy szybciej", "快一倍", "2 倍速い", "Doppelt so schnell", "El doble de rápido", "Удвічі швидше"]),
    ("Mumble didn't hear any words. Try again a little louder.", ["Mumble nie usłyszał żadnych słów. Spróbuj ponownie trochę głośniej.", "Mumble 没听到任何词语。请大声一点再试一次。", "Mumble は言葉を聞き取れませんでした。もう少し大きな声でもう一度どうぞ。", "Mumble hat keine Wörter gehört. Versuch es etwas lauter noch einmal.", "Mumble no oyó ninguna palabra. Inténtalo otra vez un poco más alto.", "Mumble не почув жодного слова. Спробуйте ще раз трохи голосніше."]),
        ("Mumble is ready", ["Mumble jest gotowy", "Mumble 已就绪", "Mumble の準備ができました", "Mumble ist bereit", "Mumble está listo", "Mumble готовий"]),
    ("Mumble's shortcut didn't start.", ["Skrót Mumble się nie uruchomił.", "Mumble 的快捷键没有启动。", "Mumble のショートカットが開始しませんでした。", "Das Kürzel von Mumble ist nicht gestartet.", "El atajo de Mumble no se inició.", "Комбінація клавіш Mumble не запустилася."]),
        ("Hold {} and say a sentence…", ["Przytrzymaj {} i powiedz zdanie…", "按住 {} 说一句话…", "{} を押したまま、ひと言話してください…", "Halte {} und sag einen Satz …", "Mantén {} y di una frase…", "Утримуйте {} і скажіть речення…"]),
    ("Hold the shortcut a little longer while you speak.", ["Przytrzymaj skrót trochę dłużej, gdy mówisz.", "说话时请把快捷键按住久一点。", "話している間、ショートカットをもう少し長く押してください。", "Halte das Kürzel beim Sprechen etwas länger.", "Mantén el atajo un poco más mientras hablas.", "Утримуйте комбінацію трохи довше, поки говорите."]),
    ("I hereby have completed Mumble onboarding.", ["Niniejszym ukończyłem wprowadzenie do Mumble.", "我在此确认已完成 Mumble 入门。", "私は Mumble のオンボーディングを修了したことをここに証します。", "Hiermit habe ich die Einführung in Mumble abgeschlossen.", "Por la presente he completado la introducción a Mumble.", "Цим я завершив знайомство з Mumble."]),
    ("It types wherever your cursor is, in any app.", ["Pisze tam, gdzie jest kursor, w każdej aplikacji.", "它会在任何应用中，把字打在光标所在处。", "どのアプリでも、カーソルの位置に入力します。", "Es tippt dort, wo dein Cursor ist, in jeder App.", "Escribe donde esté tu cursor, en cualquier app.", "Він друкує там, де курсор, у будь-якій програмі."]),
    ("Keep playing", ["Graj dalej", "继续玩", "続けて遊ぶ", "Weiterspielen", "Seguir jugando", "Грати далі"]),
    ("macOS asks once. You can change these any time in System Settings.", ["macOS zapyta raz. Możesz to zmienić w każdej chwili w Ustawieniach systemowych.", "macOS 只会询问一次。你可以随时在“系统设置”中更改。", "macOS は一度だけ確認します。システム設定でいつでも変更できます。", "macOS fragt einmal. Du kannst das jederzeit in den Systemeinstellungen ändern.", "macOS pregunta una vez. Puedes cambiarlo cuando quieras en Ajustes del Sistema.", "macOS запитає один раз. Змінити це можна будь-коли в Системних параметрах."]),
    ("messages destroyed", ["zniszczonych wiadomości", "条消息已消灭", "件のメッセージを撃破", "Nachrichten erledigt", "mensajes destruidos", "повідомлень знищено"]),
    ("Needs a restart", ["Wymaga ponownego uruchomienia", "需要重启", "再起動が必要です", "Neustart nötig", "Necesita reiniciar", "Потрібен перезапуск"]),
    ("Onboarding complete", ["Wprowadzenie ukończone", "入门完成", "オンボーディング完了", "Einführung abgeschlossen", "Introducción completada", "Знайомство завершено"]),
    ("Open Settings", ["Otwórz Ustawienia", "打开设置", "設定を開く", "Einstellungen öffnen", "Abrir Configuración", "Відкрити Параметри"]),
    ("Open System Settings", ["Otwórz Ustawienia systemowe", "打开系统设置", "システム設定を開く", "Systemeinstellungen öffnen", "Abrir Ajustes del Sistema", "Відкрити Системні параметри"]),
    ("Say anything…", ["Powiedz cokolwiek…", "随便说点什么…", "何か話してください…", "Sag irgendetwas …", "Di cualquier cosa…", "Скажіть будь-що…"]),
    ("Setting up Mumble", ["Konfigurowanie Mumble", "正在设置 Mumble", "Mumble を準備しています", "Mumble wird eingerichtet", "Configurando Mumble", "Налаштування Mumble"]),
    ("Sign by saying your name", ["Podpisz, mówiąc swoje imię", "说出你的名字来签名", "名前を言って署名します", "Unterschreib, indem du deinen Namen sagst", "Firma diciendo tu nombre", "Підпишіться, назвавши своє ім'я"]),
    ("Signed", ["Podpisano", "已签名", "署名済み", "Unterschrieben", "Firmado", "Підписано"]),
    ("speech model", ["model mowy", "语音模型", "音声モデル", "Sprachmodell", "modelo de voz", "мовну модель"]),
    ("Switch on Mumble in System Settings", ["Włącz Mumble w Ustawieniach systemowych", "在“系统设置”中打开 Mumble", "システム設定で Mumble をオンにしてください", "Schalte Mumble in den Systemeinstellungen ein", "Activa Mumble en Ajustes del Sistema", "Увімкніть Mumble у Системних параметрах"]),
        ("That didn't work. Try again.", ["Nie udało się. Spróbuj ponownie.", "没有成功，请再试一次。", "うまくいきませんでした。もう一度どうぞ。", "Das hat nicht geklappt. Versuch es noch einmal.", "No funcionó. Inténtalo de nuevo.", "Не вдалося. Спробуйте ще раз."]),
    ("That's Mumble", ["To jest Mumble", "这就是 Mumble", "これが Mumble です", "Das ist Mumble", "Eso es Mumble", "Це Mumble"]),
        ("too long", ["za długo", "太慢", "遅すぎ", "zu lange", "demasiado", "задовго"]),
    ("Try again", ["Spróbuj ponownie", "再试一次", "もう一度", "Erneut versuchen", "Reintentar", "Спробувати ще раз"]),
    ("Two panic typos later.", ["Dwie panicznie literówki później.", "慌乱中打错了两处。", "焦って 2 つ打ち間違えました。", "Zwei panische Tippfehler später.", "Dos erratas de pánico después.", "Дві панічні помилки потому."]),
    ("Typed with two typos. Spoken without any.", ["Wpisane z dwiema literówkami. Wypowiedziane bez żadnej.", "打字出了两处错，说话一处也没有。", "タイピングは 2 つの打ち間違い。話せば 1 つもありません。", "Getippt mit zwei Tippfehlern. Gesprochen ohne einen.", "Escrito con dos erratas. Dicho sin ninguna.", "Набрано з двома помилками. Сказано без жодної."]),
    ("Typing", ["Pisanie", "打字", "タイピング", "Tippen", "Escribir", "Набір"]),
    ("Typing what you said…", ["Wpisuję, co powiedziałeś…", "正在打出你说的话…", "話した内容を入力しています…", "Tippe, was du gesagt hast …", "Escribiendo lo que dijiste…", "Друкую сказане…"]),
    ("With Mumble", ["Z Mumble", "用 Mumble", "Mumble あり", "Mit Mumble", "Con Mumble", "З Mumble"]),
    ("Without Mumble", ["Bez Mumble", "不用 Mumble", "Mumble なし", "Ohne Mumble", "Sin Mumble", "Без Mumble"]),
    ("Your shortcut", ["Twój skrót", "你的快捷键", "あなたのショートカット", "Dein Kürzel", "Tu atajo", "Ваша комбінація"]),
    ("Your turn", ["Twoja kolej", "轮到你了", "あなたの番です", "Du bist dran", "Tu turno", "Ваша черга"]),
    ("Lives quietly in your menu bar", ["Działa po cichu na pasku menu", "安静地待在菜单栏中", "メニューバーで静かに待機します", "Wartet leise in deiner Menüleiste", "Vive tranquilo en tu barra de menús", "Тихо живе в рядку меню"]),
    ("Lives quietly in your system tray", ["Działa po cichu w zasobniku systemowym", "安静地待在系统托盘中", "システムトレイで静かに待機します", "Wartet leise im Infobereich", "Vive tranquilo en la bandeja del sistema", "Тихо живе в системному лотку"]),
    ("Open it again from your app launcher", ["Otworzysz go ponownie z programu uruchamiającego", "可从应用启动器中再次打开", "アプリランチャーからいつでも開けます", "Öffne es jederzeit wieder über deinen App-Starter", "Ábrelo de nuevo desde tu lanzador de apps", "Відкрийте його знову з панелі запуску програм"]),
];

#[cfg(test)]
mod tests {
    use super::TRANSLATIONS;
    use super::primary_subtag;

    #[test]
    fn system_language_tags_choose_their_interface_column() {
        use super::{
            ENGLISH, LANGUAGE_CHOICES, PORTUGUESE_COLUMN, column_for_code, column_for_language_tag,
        };
        assert_eq!(column_for_language_tag("zh-Hans-CN"), column_for_code("zh"));
        assert_eq!(column_for_language_tag("zh"), column_for_code("zh"));
        assert_eq!(column_for_language_tag("pl"), column_for_code("pl"));
        assert_eq!(column_for_language_tag("pt-BR"), PORTUGUESE_COLUMN);
        assert_eq!(column_for_language_tag("pt_PT"), PORTUGUESE_COLUMN);
        assert_eq!(column_for_code("pt"), PORTUGUESE_COLUMN);
        assert_eq!(column_for_language_tag(""), ENGLISH);
        assert!(LANGUAGE_CHOICES.contains(&(Some("pt"), "Português (Brasil)")));
        assert_eq!(column_for_language_tag("fr-FR"), super::FRENCH_COLUMN);
        assert_eq!(column_for_language_tag("fr_CA"), super::FRENCH_COLUMN);
        assert_eq!(column_for_code("fr"), super::FRENCH_COLUMN);
        assert!(LANGUAGE_CHOICES.contains(&(Some("fr"), "Français")));
    }

    #[test]
    fn traditional_chinese_locales_choose_traditional_and_others_simplified() {
        use super::{
            LANGUAGE_CHOICES, TRADITIONAL_CHINESE_COLUMN, column_for_code, column_for_language_tag,
            locale_from_env,
        };
        for tag in [
            "zh-Hant-TW",
            "zh-Hant",
            "zh-TW",
            "zh_HK.UTF-8",
            "zh-MO",
            "zh-Hant-CN",
        ] {
            assert_eq!(
                column_for_language_tag(tag),
                TRADITIONAL_CHINESE_COLUMN,
                "{tag}"
            );
        }
        for tag in [
            "zh-Hans-CN",
            "zh-CN",
            "zh-SG",
            "zh-Hans-HK",
            "zh_CN.UTF-8",
            "zh",
        ] {
            assert_eq!(column_for_language_tag(tag), column_for_code("zh"), "{tag}");
        }
        assert_eq!(column_for_code("zh-Hant"), TRADITIONAL_CHINESE_COLUMN);
        assert!(LANGUAGE_CHOICES.contains(&(Some("zh"), "简体中文")));
        assert!(LANGUAGE_CHOICES.contains(&(Some("zh-Hant"), "繁體中文")));
        let locale = locale_from_env(None, Some("zh_TW.UTF-8"), Some("en_US.UTF-8"));
        assert_eq!(locale, "zh_TW.UTF-8");
        assert_eq!(column_for_language_tag(&locale), TRADITIONAL_CHINESE_COLUMN);
        assert_eq!(locale_from_env(Some("C"), None, None), "");
    }

    #[test]
    fn traditional_chinese_translates_with_its_column() {
        super::set_current(super::TRADITIONAL_CHINESE_COLUMN);
        assert_eq!(super::tr("Don't switch"), "不切換");
        assert_eq!(super::tr("Not installed"), "未安裝");
        assert_eq!(super::tr_fill("Remove {}?", "x"), "移除 x？");
        super::set_current(super::ENGLISH);
        assert_eq!(super::tr("Don't switch"), "Don't switch");
    }

    #[test]
    fn french_translates_with_its_column() {
        super::set_current(super::FRENCH_COLUMN);
        assert_eq!(super::tr("Don't switch"), "Ne pas changer");
        assert_eq!(super::tr_fill("Remove {}?", "x"), "Supprimer x ?");
        super::set_current(super::ENGLISH);
        assert_eq!(super::tr("Don't switch"), "Don't switch");
    }

    #[test]
    fn portuguese_translates_with_its_column() {
        super::set_current(super::PORTUGUESE_COLUMN);
        assert_eq!(super::tr("Labs"), "Labs");
        assert_eq!(super::tr("Don't switch"), "Não trocar");
        assert_eq!(super::tr_fill("Remove {}?", "x"), "Remover x?");
        super::set_current(super::ENGLISH);
        assert_eq!(super::tr("Don't switch"), "Don't switch");
    }

    #[test]
    fn preferred_language_tags_reduce_to_their_primary_subtag() {
        assert_eq!(primary_subtag("pt-BR").as_deref(), Some("pt"));
        assert_eq!(primary_subtag("zh-Hans-CN").as_deref(), Some("zh"));
        assert_eq!(primary_subtag("en").as_deref(), Some("en"));
        assert_eq!(primary_subtag(""), None);
    }

    #[test]
    fn templates_keep_their_placeholder_in_every_language() {
        for (english, translated) in TRANSLATIONS {
            if english.contains("{}") {
                for translation in translated {
                    assert!(
                        translation.contains("{}"),
                        "translation of {english:?} lost its placeholder: {translation:?}"
                    );
                }
            }
        }
    }

    /// Every source that translates, on every OS.
    const SOURCES: &[&str] = &[
        include_str!("opencode_catalog.rs"),
        include_str!("../rune_ui.rs"),
        include_str!("../rune_ui/activity.rs"),
        include_str!("../rune_ui/explainer.rs"),
        include_str!("../rune_ui/fun.rs"),
        include_str!("../rune_ui/general.rs"),
        include_str!("../rune_ui/history.rs"),
        include_str!("../rune_ui/hud_lab.rs"),
        include_str!("../rune_ui/labs.rs"),
        include_str!("../rune_ui/linux_host.rs"),
        include_str!("../rune_ui/list_detail.rs"),
        include_str!("../rune_ui/lists.rs"),
        include_str!("../rune_ui/macos_host.rs"),
        include_str!("../rune_ui/models.rs"),
        include_str!("../rune_ui/modes.rs"),
        include_str!("../rune_ui/onboarding_kit/first_run.rs"),
        include_str!("../rune_ui/onboarding_kit/flows.rs"),
        include_str!("../rune_ui/onboarding_kit/game.rs"),
        include_str!("../rune_ui/onboarding_kit/landing.rs"),
        include_str!("../rune_ui/onboarding_kit/live.rs"),
        include_str!("../rune_ui/onboarding_kit/permissions.rs"),
        include_str!("../rune_ui/onboarding_kit/stories.rs"),
        include_str!("../rune_ui/onboarding_kit/versus.rs"),
        include_str!("../rune_ui/output.rs"),
        include_str!("../rune_ui/platform_settings.rs"),
        include_str!("../rune_ui/settings.rs"),
        include_str!("../rune_ui/shortcut_mode.rs"),
        include_str!("../rune_ui/sound_sets.rs"),
        include_str!("../rune_ui/vocabulary.rs"),
        include_str!("../rune_ui/windows_host.rs"),
        include_str!("tray_menu.rs"),
        include_str!("../windows/app.rs"),
        include_str!("../windows/rune.rs"),
    ];

    /// The string literals passed to `tr`, `tr_fill`, and `tr_named`.
    fn translated_literals(source: &str) -> Vec<String> {
        let mut literals = Vec::new();
        for call in ["tr(", "tr_fill(", "tr_named("] {
            for (start, _) in source.match_indices(call) {
                let preceded_by_ident = source[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|character| character.is_alphanumeric() || character == '_');
                if preceded_by_ident {
                    continue;
                }
                let rest = source[start + call.len()..].trim_start();
                let Some(rest) = rest.strip_prefix('"') else {
                    continue;
                };
                let mut literal = String::new();
                let mut characters = rest.chars();
                while let Some(character) = characters.next() {
                    match character {
                        '"' => break,
                        '\\' => literal.extend(characters.next()),
                        other => literal.push(other),
                    }
                }
                literals.push(literal);
            }
        }
        literals
    }

    #[test]
    fn every_translated_string_has_every_language() {
        let table: std::collections::HashMap<_, _> = TRANSLATIONS.iter().copied().collect();
        let mut checked = 0;
        for source in SOURCES {
            for literal in translated_literals(source) {
                let translations = table
                    .get(literal.as_str())
                    .unwrap_or_else(|| panic!("{literal:?} has no translations"));
                assert!(
                    translations
                        .iter()
                        .all(|translation| !translation.is_empty()),
                    "{literal:?} is missing a language"
                );
                checked += 1;
            }
        }
        assert!(checked > 300, "only {checked} translated strings found");
    }

    /// The sound picker translates each set's name and character and each
    /// cue's name, which are not literals at the call.
    #[test]
    fn every_sound_set_string_has_every_language() {
        use crate::feedback::{Cue, SoundSet};
        let table: std::collections::HashMap<_, _> = TRANSLATIONS.iter().copied().collect();
        let strings = SoundSet::ALL
            .into_iter()
            .flat_map(|set| [set.name(), set.character()])
            .chain(Cue::ALL.into_iter().map(Cue::name));
        for english in strings {
            let translations = table
                .get(english)
                .unwrap_or_else(|| panic!("{english:?} has no translations"));
            assert!(
                translations
                    .iter()
                    .all(|translation| !translation.is_empty())
            );
        }
    }

    #[test]
    fn named_placeholders_survive_translation() {
        for (english, translated) in TRANSLATIONS {
            let names = english
                .split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}'))
                .map(|(name, _)| name)
                .filter(|name| !name.is_empty());
            for name in names {
                for translation in translated {
                    assert!(
                        translation.contains(&format!("{{{name}}}")),
                        "translation of {english:?} lost {{{name}}}: {translation:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (english, _) in TRANSLATIONS {
            assert!(
                seen.insert(english),
                "duplicate translation key {english:?}"
            );
        }
    }
}
