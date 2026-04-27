use spin::{Lazy, Mutex};

use crate::fs::{self, FILESYSTEM};
use crate::hal::device::KeyboardLayout;

const UI_PREFS_PATH: &str = "/etc/ui.pref";

static UI_PREFERENCES: Lazy<Mutex<UiPreferences>> =
    Lazy::new(|| Mutex::new(UiPreferences::default()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Portuguese,
    German,
    Russian,
    Japanese,
    MandarinChinese,
    Spanish,
    French,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeZone {
    Utc,
    UtcMinus03,
    UtcMinus04,
    UtcMinus05,
    UtcMinus06,
    UtcMinus07,
    UtcMinus08,
    UtcPlus01,
    UtcPlus02,
    UtcPlus03,
    UtcPlus04,
    UtcPlus0530,
    UtcPlus08,
    UtcPlus09,
    UtcPlus10,
    UtcPlus12,
}

#[derive(Debug, Clone, Copy)]
pub struct UiPreferences {
    pub language: Language,
    pub keyboard_layout: KeyboardLayout,
    pub timezone: TimeZone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextId {
    PreferencesTitle,
    PreferencesSubtitle,
    PreferencesContinue,
    PreferencesLanguage,
    PreferencesKeyboard,
    PreferencesLayoutReady,
    PreferencesLayoutStaged,
    SetupTitle,
    SetupSubtitle,
    LoginTitle,
    LoginSubtitle,
    UsernameLabel,
    PasswordLabel,
    ConfirmLabel,
    PasswordMismatch,
    AccountCreated,
    AccountCreateFailed,
    UserNotFound,
    WrongPassword,
    AccountDisabled,
    AuthFailed,
    WelcomeBack,
    LastLogin,
    TooManyAttempts,
    SessionPreparing,
    SessionReadyTitle,
    SessionReadyFirstBoot,
    SessionReadyLogin,
    SessionReadyHint,
    CurrentLanguage,
    CurrentLayout,
    PreferencesTimezoneLater,
    FullLocalizationStaged,
    KeyboardSupportStaged,
    LastLoginRecorded,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            language: Language::English,
            keyboard_layout: KeyboardLayout::UsQwerty,
            timezone: TimeZone::Utc,
        }
    }
}

impl Language {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Portuguese => "pt",
            Self::German => "de",
            Self::Russian => "ru",
            Self::Japanese => "ja",
            Self::MandarinChinese => "zh",
            Self::Spanish => "es",
            Self::French => "fr",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Portuguese => "Portugues",
            Self::German => "Deutsch",
            Self::Russian => "Russian",
            Self::Japanese => "Japanese",
            Self::MandarinChinese => "Chinese",
            Self::Spanish => "Espanol",
            Self::French => "Francais",
        }
    }

    #[must_use]
    pub fn fully_localized(self) -> bool {
        matches!(
            self,
            Self::English | Self::Portuguese | Self::German | Self::Spanish | Self::French
        )
    }
}

impl TimeZone {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Utc => "utc",
            Self::UtcMinus03 => "utc-03",
            Self::UtcMinus04 => "utc-04",
            Self::UtcMinus05 => "utc-05",
            Self::UtcMinus06 => "utc-06",
            Self::UtcMinus07 => "utc-07",
            Self::UtcMinus08 => "utc-08",
            Self::UtcPlus01 => "utc+01",
            Self::UtcPlus02 => "utc+02",
            Self::UtcPlus03 => "utc+03",
            Self::UtcPlus04 => "utc+04",
            Self::UtcPlus0530 => "utc+0530",
            Self::UtcPlus08 => "utc+08",
            Self::UtcPlus09 => "utc+09",
            Self::UtcPlus10 => "utc+10",
            Self::UtcPlus12 => "utc+12",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Utc => "UTC+00:00",
            Self::UtcMinus03 => "UTC-03:00",
            Self::UtcMinus04 => "UTC-04:00",
            Self::UtcMinus05 => "UTC-05:00",
            Self::UtcMinus06 => "UTC-06:00",
            Self::UtcMinus07 => "UTC-07:00",
            Self::UtcMinus08 => "UTC-08:00",
            Self::UtcPlus01 => "UTC+01:00",
            Self::UtcPlus02 => "UTC+02:00",
            Self::UtcPlus03 => "UTC+03:00",
            Self::UtcPlus04 => "UTC+04:00",
            Self::UtcPlus0530 => "UTC+05:30",
            Self::UtcPlus08 => "UTC+08:00",
            Self::UtcPlus09 => "UTC+09:00",
            Self::UtcPlus10 => "UTC+10:00",
            Self::UtcPlus12 => "UTC+12:00",
        }
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Utc => "UTC fixed offset / lab default",
            Self::UtcMinus03 => "BRT / ART / FKST (no DST rules)",
            Self::UtcMinus04 => "AST / AMT / VET (no DST rules)",
            Self::UtcMinus05 => "EST / COT / PET (no DST rules)",
            Self::UtcMinus06 => "CST US-Central / MEX (no DST rules)",
            Self::UtcMinus07 => "MST US-Mountain (no DST rules)",
            Self::UtcMinus08 => "PST US-Pacific (no DST rules)",
            Self::UtcPlus01 => "CET / WAT (no DST rules)",
            Self::UtcPlus02 => "EET / CAT / SAST (no DST rules)",
            Self::UtcPlus03 => "MSK / EAT / AST-Arabia (no DST rules)",
            Self::UtcPlus04 => "GST Gulf / Samara (no DST rules)",
            Self::UtcPlus0530 => "IST India (no DST rules)",
            Self::UtcPlus08 => "CST China / AWST / SGT (no DST rules)",
            Self::UtcPlus09 => "JST / KST (no DST rules)",
            Self::UtcPlus10 => "AEST / PGT (no DST rules)",
            Self::UtcPlus12 => "NZST / FJT (no DST rules)",
        }
    }

    #[must_use]
    pub fn offset_minutes(self) -> i16 {
        match self {
            Self::Utc => 0,
            Self::UtcMinus03 => -180,
            Self::UtcMinus04 => -240,
            Self::UtcMinus05 => -300,
            Self::UtcMinus06 => -360,
            Self::UtcMinus07 => -420,
            Self::UtcMinus08 => -480,
            Self::UtcPlus01 => 60,
            Self::UtcPlus02 => 120,
            Self::UtcPlus03 => 180,
            Self::UtcPlus04 => 240,
            Self::UtcPlus0530 => 330,
            Self::UtcPlus08 => 480,
            Self::UtcPlus09 => 540,
            Self::UtcPlus10 => 600,
            Self::UtcPlus12 => 720,
        }
    }
}

pub fn init() {
    let preferences = load_preferences_from_fs().unwrap_or_default();
    let applied_layout = apply_layout_or_default(preferences.keyboard_layout);
    *UI_PREFERENCES.lock() = UiPreferences {
        language: preferences.language,
        keyboard_layout: applied_layout,
        timezone: preferences.timezone,
    };
}

#[must_use]
pub fn preferences() -> UiPreferences {
    *UI_PREFERENCES.lock()
}

#[must_use]
pub fn language() -> Language {
    preferences().language
}

#[must_use]
pub fn keyboard_layout() -> KeyboardLayout {
    preferences().keyboard_layout
}

#[must_use]
pub fn timezone() -> TimeZone {
    preferences().timezone
}

pub fn set_language(language: Language) {
    UI_PREFERENCES.lock().language = language;
}

pub fn set_language_by_code(code: &str) -> Result<Language, &'static str> {
    let language = parse_language(code).ok_or("unknown language")?;
    set_language(language);
    Ok(language)
}

pub fn set_keyboard_layout(layout: KeyboardLayout) -> Result<(), &'static str> {
    crate::hal::input::set_layout(layout)?;
    UI_PREFERENCES.lock().keyboard_layout = layout;
    Ok(())
}

pub fn set_timezone(timezone: TimeZone) {
    UI_PREFERENCES.lock().timezone = timezone;
}

pub fn set_timezone_by_code(code: &str) -> Result<TimeZone, &'static str> {
    let timezone = parse_timezone(code).ok_or("unknown timezone")?;
    set_timezone(timezone);
    Ok(timezone)
}

pub fn save_preferences() -> Result<(), fs::FsError> {
    let preferences = preferences();
    let payload = alloc::format!(
        "language={}\nlayout={}\ntimezone={}\n",
        preferences.language.code(),
        layout_code(preferences.keyboard_layout),
        preferences.timezone.code()
    );
    FILESYSTEM
        .lock()
        .write_system(UI_PREFS_PATH, payload.as_bytes(), false)
}

#[must_use]
pub fn supported_languages() -> &'static [(Language, &'static str, bool)] {
    &[
        (Language::English, "English", true),
        (Language::Portuguese, "Portugues", true),
        (Language::German, "Deutsch", true),
        (Language::Spanish, "Espanol", true),
        (Language::French, "Francais", true),
        (Language::Russian, "Russian", false),
        (Language::Japanese, "Japanese", false),
        (Language::MandarinChinese, "Chinese", false),
    ]
}

#[must_use]
pub fn keyboard_layout_choices() -> &'static [(&'static str, KeyboardLayout, bool)] {
    &[
        ("en-US / US QWERTY", KeyboardLayout::UsQwerty, true),
        ("pt-BR / ABNT2", KeyboardLayout::BrazilAbnt2, false),
        ("de-DE / QWERTZ", KeyboardLayout::German, false),
        ("es-ES / ES", KeyboardLayout::Spanish, false),
        ("fr-FR / AZERTY", KeyboardLayout::French, false),
        ("ru-RU / staged", KeyboardLayout::Custom, false),
        ("ja-JP / staged", KeyboardLayout::Japanese, false),
        ("zh-CN / staged", KeyboardLayout::Custom, false),
    ]
}

#[must_use]
pub fn supported_timezones() -> &'static [TimeZone] {
    &[
        TimeZone::UtcMinus08,
        TimeZone::UtcMinus07,
        TimeZone::UtcMinus06,
        TimeZone::UtcMinus05,
        TimeZone::UtcMinus04,
        TimeZone::UtcMinus03,
        TimeZone::Utc,
        TimeZone::UtcPlus01,
        TimeZone::UtcPlus02,
        TimeZone::UtcPlus03,
        TimeZone::UtcPlus04,
        TimeZone::UtcPlus0530,
        TimeZone::UtcPlus08,
        TimeZone::UtcPlus09,
        TimeZone::UtcPlus10,
        TimeZone::UtcPlus12,
    ]
}

#[must_use]
pub fn layout_label(layout: KeyboardLayout) -> &'static str {
    match layout {
        KeyboardLayout::UsQwerty => "en-US",
        KeyboardLayout::BrazilAbnt2 => "pt-BR",
        KeyboardLayout::German => "de-DE",
        KeyboardLayout::Spanish => "es-ES",
        KeyboardLayout::French => "fr-FR",
        KeyboardLayout::Japanese => "ja-JP",
        KeyboardLayout::UkQwerty => "en-UK",
        KeyboardLayout::Custom => "custom",
    }
}

#[must_use]
pub fn text(id: TextId) -> &'static str {
    match language() {
        Language::Portuguese => text_portuguese(id),
        Language::German => text_german(id),
        Language::Spanish => text_spanish(id),
        Language::French => text_french(id),
        Language::English | Language::Russian | Language::Japanese | Language::MandarinChinese => {
            text_english(id)
        }
    }
}

#[must_use]
pub fn localization_status_note() -> Option<&'static str> {
    (!language().fully_localized()).then(|| text(TextId::FullLocalizationStaged))
}

#[must_use]
pub fn timezone_status_note() -> &'static str {
    "WarOS currently stores a manual fixed UTC offset only. Automatic timezone detection, DST rules, RTC sync, and NTP sync are not implemented."
}

fn apply_layout_or_default(layout: KeyboardLayout) -> KeyboardLayout {
    if crate::hal::input::set_layout(layout).is_ok() {
        layout
    } else {
        let fallback = KeyboardLayout::UsQwerty;
        let _ = crate::hal::input::set_layout(fallback);
        fallback
    }
}

fn load_preferences_from_fs() -> Option<UiPreferences> {
    let text_bytes = {
        let filesystem = FILESYSTEM.lock();
        filesystem.read(UI_PREFS_PATH).ok()?.to_vec()
    };
    let text = core::str::from_utf8(&text_bytes).ok()?;

    let mut language = UiPreferences::default().language;
    let mut keyboard_layout = UiPreferences::default().keyboard_layout;
    let mut timezone = UiPreferences::default().timezone;

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "language" => {
                language = parse_language(value.trim()).unwrap_or(language);
            }
            "layout" => {
                keyboard_layout = parse_layout(value.trim()).unwrap_or(keyboard_layout);
            }
            "timezone" => {
                timezone = parse_timezone(value.trim()).unwrap_or(timezone);
            }
            _ => {}
        }
    }

    Some(UiPreferences {
        language,
        keyboard_layout,
        timezone,
    })
}

fn parse_language(code: &str) -> Option<Language> {
    match code {
        "en" => Some(Language::English),
        "pt" => Some(Language::Portuguese),
        "de" => Some(Language::German),
        "ru" => Some(Language::Russian),
        "ja" => Some(Language::Japanese),
        "zh" => Some(Language::MandarinChinese),
        "es" => Some(Language::Spanish),
        "fr" => Some(Language::French),
        _ => None,
    }
}

fn parse_timezone(code: &str) -> Option<TimeZone> {
    match code {
        "utc" | "utc+00" | "utc+00:00" => Some(TimeZone::Utc),
        "utc-03" | "utc-03:00" => Some(TimeZone::UtcMinus03),
        "utc-04" | "utc-04:00" => Some(TimeZone::UtcMinus04),
        "utc-05" | "utc-05:00" => Some(TimeZone::UtcMinus05),
        "utc-06" | "utc-06:00" => Some(TimeZone::UtcMinus06),
        "utc-07" | "utc-07:00" => Some(TimeZone::UtcMinus07),
        "utc-08" | "utc-08:00" => Some(TimeZone::UtcMinus08),
        "utc+01" | "utc+01:00" => Some(TimeZone::UtcPlus01),
        "utc+02" | "utc+02:00" => Some(TimeZone::UtcPlus02),
        "utc+03" | "utc+03:00" => Some(TimeZone::UtcPlus03),
        "utc+04" | "utc+04:00" => Some(TimeZone::UtcPlus04),
        "utc+0530" | "utc+05:30" => Some(TimeZone::UtcPlus0530),
        "utc+08" | "utc+08:00" => Some(TimeZone::UtcPlus08),
        "utc+09" | "utc+09:00" => Some(TimeZone::UtcPlus09),
        "utc+10" | "utc+10:00" => Some(TimeZone::UtcPlus10),
        "utc+12" | "utc+12:00" => Some(TimeZone::UtcPlus12),
        _ => None,
    }
}

fn parse_layout(code: &str) -> Option<KeyboardLayout> {
    match code {
        "us" => Some(KeyboardLayout::UsQwerty),
        "br" => Some(KeyboardLayout::BrazilAbnt2),
        "de" => Some(KeyboardLayout::German),
        "es" => Some(KeyboardLayout::Spanish),
        "fr" => Some(KeyboardLayout::French),
        "jp" => Some(KeyboardLayout::Japanese),
        "uk" => Some(KeyboardLayout::UkQwerty),
        "custom" => Some(KeyboardLayout::Custom),
        _ => None,
    }
}

fn layout_code(layout: KeyboardLayout) -> &'static str {
    match layout {
        KeyboardLayout::UsQwerty => "us",
        KeyboardLayout::BrazilAbnt2 => "br",
        KeyboardLayout::German => "de",
        KeyboardLayout::Spanish => "es",
        KeyboardLayout::French => "fr",
        KeyboardLayout::Japanese => "jp",
        KeyboardLayout::UkQwerty => "uk",
        KeyboardLayout::Custom => "custom",
    }
}

fn text_english(id: TextId) -> &'static str {
    match id {
        TextId::PreferencesTitle => "Access Preferences",
        TextId::PreferencesSubtitle => "Choose language and keyboard layout before continuing.",
        TextId::PreferencesContinue => "Continue",
        TextId::PreferencesLanguage => "System language",
        TextId::PreferencesKeyboard => "Keyboard layout",
        TextId::PreferencesLayoutReady => "ready",
        TextId::PreferencesLayoutStaged => "staged",
        TextId::SetupTitle => "First Time Setup",
        TextId::SetupSubtitle => "Welcome to WarOS. Create your admin account.",
        TextId::LoginTitle => "Secure Login",
        TextId::LoginSubtitle => "Sign in to continue.",
        TextId::UsernameLabel => "Username",
        TextId::PasswordLabel => "Password",
        TextId::ConfirmLabel => "Confirm",
        TextId::PasswordMismatch => "Passwords do not match.",
        TextId::AccountCreated => "Account created. You are now the system administrator.",
        TextId::AccountCreateFailed => "Failed to create account",
        TextId::UserNotFound => "User not found.",
        TextId::WrongPassword => "Incorrect password.",
        TextId::AccountDisabled => "Account disabled.",
        TextId::AuthFailed => "Authentication failed.",
        TextId::WelcomeBack => "Welcome back",
        TextId::LastLogin => "Last login",
        TextId::TooManyAttempts => "Too many failed attempts. Waiting 30 seconds...",
        TextId::SessionPreparing => "Preparing your session...",
        TextId::SessionReadyTitle => "System Ready",
        TextId::SessionReadyFirstBoot => "Setup complete. Administrator session is ready.",
        TextId::SessionReadyLogin => "Session ready.",
        TextId::SessionReadyHint => {
            "Shell controls: Up/Down recalls history, Esc clears the line, 'help' lists commands."
        }
        TextId::CurrentLanguage => "Language",
        TextId::CurrentLayout => "Layout",
        TextId::PreferencesTimezoneLater => {
            "Timezone preference is configured later in the shell with 'timezone list'."
        }
        TextId::FullLocalizationStaged => {
            "Full console localization for this language is staged; WarOS is using English UI for now."
        }
        TextId::KeyboardSupportStaged => {
            "Keyboard layouts ru-RU, ja-JP, zh-CN are staged for a future shared scan-code mapper."
        }
        TextId::LastLoginRecorded => "previous login recorded",
    }
}

fn text_portuguese(id: TextId) -> &'static str {
    match id {
        TextId::PreferencesTitle => "Preferencias de Acesso",
        TextId::PreferencesSubtitle => "Escolha idioma e layout do teclado antes de continuar.",
        TextId::PreferencesContinue => "Continuar",
        TextId::PreferencesLanguage => "Idioma do sistema",
        TextId::PreferencesKeyboard => "Layout do teclado",
        TextId::PreferencesLayoutReady => "pronto",
        TextId::PreferencesLayoutStaged => "em preparo",
        TextId::SetupTitle => "Primeira Configuracao",
        TextId::SetupSubtitle => "Bem-vindo ao WarOS. Crie sua conta de administrador.",
        TextId::LoginTitle => "Login Seguro",
        TextId::LoginSubtitle => "Entre para continuar.",
        TextId::UsernameLabel => "Usuario",
        TextId::PasswordLabel => "Senha",
        TextId::ConfirmLabel => "Confirmar",
        TextId::PasswordMismatch => "As senhas nao coincidem.",
        TextId::AccountCreated => "Conta criada. Voce agora e o administrador do sistema.",
        TextId::AccountCreateFailed => "Falha ao criar a conta",
        TextId::UserNotFound => "Usuario nao encontrado.",
        TextId::WrongPassword => "Senha incorreta.",
        TextId::AccountDisabled => "Conta desativada.",
        TextId::AuthFailed => "Falha de autenticacao.",
        TextId::WelcomeBack => "Bem-vindo de volta",
        TextId::LastLogin => "Ultimo login",
        TextId::TooManyAttempts => "Muitas tentativas falharam. Aguardando 30 segundos...",
        TextId::SessionPreparing => "Preparando sua sessao...",
        TextId::SessionReadyTitle => "Sistema Pronto",
        TextId::SessionReadyFirstBoot => "Configuracao concluida. A sessao administrativa esta pronta.",
        TextId::SessionReadyLogin => "Sessao pronta.",
        TextId::SessionReadyHint => {
            "Atalhos do shell: Up/Down recupera historico, Esc limpa a linha, 'help' mostra comandos."
        }
        TextId::CurrentLanguage => "Idioma",
        TextId::CurrentLayout => "Layout",
        TextId::PreferencesTimezoneLater => {
            "A preferencia de fuso horario e configurada depois no shell com 'timezone list'."
        }
        TextId::FullLocalizationStaged => {
            "A localizacao completa para este idioma ainda esta em preparo; a interface segue em ingles."
        }
        TextId::KeyboardSupportStaged => {
            "Os layouts ru-RU, ja-JP, zh-CN estao em preparo para um mapeador compartilhado de scan-codes."
        }
        TextId::LastLoginRecorded => "login anterior registrado",
    }
}

fn text_german(id: TextId) -> &'static str {
    match id {
        TextId::PreferencesTitle => "Zugangsoptionen",
        TextId::PreferencesSubtitle => {
            "Waehlen Sie Sprache und Tastaturlayout vor dem Fortfahren."
        }
        TextId::PreferencesContinue => "Weiter",
        TextId::PreferencesLanguage => "Systemsprache",
        TextId::PreferencesKeyboard => "Tastaturlayout",
        TextId::PreferencesLayoutReady => "bereit",
        TextId::PreferencesLayoutStaged => "gestuft",
        TextId::SetupTitle => "Ersteinrichtung",
        TextId::SetupSubtitle => "Willkommen bei WarOS. Erstellen Sie Ihr Administratorkonto.",
        TextId::LoginTitle => "Sichere Anmeldung",
        TextId::LoginSubtitle => "Melden Sie sich an, um fortzufahren.",
        TextId::UsernameLabel => "Benutzername",
        TextId::PasswordLabel => "Passwort",
        TextId::ConfirmLabel => "Bestaetigen",
        TextId::PasswordMismatch => "Die Passwoerter stimmen nicht ueberein.",
        TextId::AccountCreated => {
            "Konto erstellt. Sie sind jetzt der Systemadministrator."
        }
        TextId::AccountCreateFailed => "Konto konnte nicht erstellt werden",
        TextId::UserNotFound => "Benutzer wurde nicht gefunden.",
        TextId::WrongPassword => "Falsches Passwort.",
        TextId::AccountDisabled => "Konto ist deaktiviert.",
        TextId::AuthFailed => "Authentifizierung fehlgeschlagen.",
        TextId::WelcomeBack => "Willkommen zurueck",
        TextId::LastLogin => "Letzte Anmeldung",
        TextId::TooManyAttempts => "Zu viele Fehlversuche. Warte 30 Sekunden...",
        TextId::SessionPreparing => "Ihre Sitzung wird vorbereitet...",
        TextId::SessionReadyTitle => "System bereit",
        TextId::SessionReadyFirstBoot => {
            "Einrichtung abgeschlossen. Die Administratorsitzung ist bereit."
        }
        TextId::SessionReadyLogin => "Sitzung bereit.",
        TextId::SessionReadyHint => {
            "Shell-Steuerung: Up/Down ruft Verlauf ab, Esc leert die Zeile, 'help' zeigt Befehle."
        }
        TextId::CurrentLanguage => "Sprache",
        TextId::CurrentLayout => "Layout",
        TextId::PreferencesTimezoneLater => {
            "Die Zeitzonenpraferenz wird spaeter in der Shell mit 'timezone list' gesetzt."
        }
        TextId::FullLocalizationStaged => {
            "Die volle Konsolenlokalisierung fuer diese Sprache ist gestuft; WarOS nutzt vorerst die englische UI."
        }
        TextId::KeyboardSupportStaged => {
            "Tastaturlayouts ru-RU, ja-JP, zh-CN sind fuer einen spaeteren gemeinsamen Scan-Code-Mapper gestuft."
        }
        TextId::LastLoginRecorded => "vorherige Anmeldung registriert",
    }
}

fn text_spanish(id: TextId) -> &'static str {
    match id {
        TextId::PreferencesTitle => "Preferencias de Acceso",
        TextId::PreferencesSubtitle => {
            "Elija idioma y distribucion de teclado antes de continuar."
        }
        TextId::PreferencesContinue => "Continuar",
        TextId::PreferencesLanguage => "Idioma del sistema",
        TextId::PreferencesKeyboard => "Distribucion del teclado",
        TextId::PreferencesLayoutReady => "listo",
        TextId::PreferencesLayoutStaged => "en preparacion",
        TextId::SetupTitle => "Configuracion Inicial",
        TextId::SetupSubtitle => {
            "Bienvenido a WarOS. Cree su cuenta de administrador."
        }
        TextId::LoginTitle => "Inicio de Sesion Seguro",
        TextId::LoginSubtitle => "Inicie sesion para continuar.",
        TextId::UsernameLabel => "Usuario",
        TextId::PasswordLabel => "Contrasena",
        TextId::ConfirmLabel => "Confirmar",
        TextId::PasswordMismatch => "Las contrasenas no coinciden.",
        TextId::AccountCreated => {
            "Cuenta creada. Ahora es el administrador del sistema."
        }
        TextId::AccountCreateFailed => "No se pudo crear la cuenta",
        TextId::UserNotFound => "Usuario no encontrado.",
        TextId::WrongPassword => "Contrasena incorrecta.",
        TextId::AccountDisabled => "Cuenta deshabilitada.",
        TextId::AuthFailed => "Autenticacion fallida.",
        TextId::WelcomeBack => "Bienvenido de nuevo",
        TextId::LastLogin => "Ultimo acceso",
        TextId::TooManyAttempts => "Demasiados intentos fallidos. Esperando 30 segundos...",
        TextId::SessionPreparing => "Preparando su sesion...",
        TextId::SessionReadyTitle => "Sistema Listo",
        TextId::SessionReadyFirstBoot => {
            "Configuracion completada. La sesion de administrador esta lista."
        }
        TextId::SessionReadyLogin => "Sesion lista.",
        TextId::SessionReadyHint => {
            "Controles del shell: Up/Down recupera historial, Esc limpia la linea, 'help' muestra comandos."
        }
        TextId::CurrentLanguage => "Idioma",
        TextId::CurrentLayout => "Layout",
        TextId::PreferencesTimezoneLater => {
            "La preferencia de zona horaria se configura despues en el shell con 'timezone list'."
        }
        TextId::FullLocalizationStaged => {
            "La localizacion completa para este idioma esta en preparacion; WarOS usa la interfaz en ingles por ahora."
        }
        TextId::KeyboardSupportStaged => {
            "Los layouts ru-RU, ja-JP, zh-CN quedan en preparacion para un mapeador compartido de scan-codes."
        }
        TextId::LastLoginRecorded => "inicio de sesion anterior registrado",
    }
}

fn text_french(id: TextId) -> &'static str {
    match id {
        TextId::PreferencesTitle => "Preferences d'Acces",
        TextId::PreferencesSubtitle => {
            "Choisissez la langue et le clavier avant de continuer."
        }
        TextId::PreferencesContinue => "Continuer",
        TextId::PreferencesLanguage => "Langue du systeme",
        TextId::PreferencesKeyboard => "Disposition du clavier",
        TextId::PreferencesLayoutReady => "pret",
        TextId::PreferencesLayoutStaged => "en attente",
        TextId::SetupTitle => "Premiere Configuration",
        TextId::SetupSubtitle => {
            "Bienvenue sur WarOS. Creez votre compte administrateur."
        }
        TextId::LoginTitle => "Connexion Securisee",
        TextId::LoginSubtitle => "Connectez-vous pour continuer.",
        TextId::UsernameLabel => "Utilisateur",
        TextId::PasswordLabel => "Mot de passe",
        TextId::ConfirmLabel => "Confirmer",
        TextId::PasswordMismatch => "Les mots de passe ne correspondent pas.",
        TextId::AccountCreated => {
            "Compte cree. Vous etes maintenant l'administrateur du systeme."
        }
        TextId::AccountCreateFailed => "Echec de creation du compte",
        TextId::UserNotFound => "Utilisateur introuvable.",
        TextId::WrongPassword => "Mot de passe incorrect.",
        TextId::AccountDisabled => "Compte desactive.",
        TextId::AuthFailed => "Echec d'authentification.",
        TextId::WelcomeBack => "Bon retour",
        TextId::LastLogin => "Derniere connexion",
        TextId::TooManyAttempts => "Trop d'echecs. Attente de 30 secondes...",
        TextId::SessionPreparing => "Preparation de votre session...",
        TextId::SessionReadyTitle => "Systeme Pret",
        TextId::SessionReadyFirstBoot => {
            "Configuration terminee. La session administrateur est prete."
        }
        TextId::SessionReadyLogin => "Session prete.",
        TextId::SessionReadyHint => {
            "Controles du shell: Up/Down rappelle l'historique, Esc efface la ligne, 'help' liste les commandes."
        }
        TextId::CurrentLanguage => "Langue",
        TextId::CurrentLayout => "Disposition",
        TextId::PreferencesTimezoneLater => {
            "La preference de fuseau horaire se configure plus tard dans le shell avec 'timezone list'."
        }
        TextId::FullLocalizationStaged => {
            "La localisation complete pour cette langue reste en attente; WarOS utilise pour l'instant l'interface anglaise."
        }
        TextId::KeyboardSupportStaged => {
            "Les claviers ru-RU, ja-JP, zh-CN restent en attente d'un mappeur partage de scan-codes."
        }
        TextId::LastLoginRecorded => "connexion precedente enregistree",
    }
}
