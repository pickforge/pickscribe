use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pickscribe::config::AppConfig;
use sentry::{ClientOptions, Envelope, Transport, TransportFactory};
use tauri::{AppHandle, Emitter};

const EVENT_CONFIG: &str = "pickscribe://config";

static MUTATION_LOCK: Mutex<()> = Mutex::new(());
static TELEMETRY_ENABLED: AtomicBool = AtomicBool::new(false);
static SENTRY_CLIENT: OnceLock<Arc<sentry::Client>> = OnceLock::new();
static MINIDUMP_GUARD: Mutex<Option<tauri_plugin_sentry::minidump::Handle>> = Mutex::new(None);

struct GatedTransport {
    inner: Arc<dyn Transport>,
}

impl Transport for GatedTransport {
    fn send_envelope(&self, envelope: Envelope) {
        if telemetry_enabled() {
            self.inner.send_envelope(envelope);
        }
    }

    fn flush(&self, timeout: Duration) -> bool {
        if telemetry_enabled() {
            self.inner.flush(timeout)
        } else {
            true
        }
    }

    fn shutdown(&self, timeout: Duration) -> bool {
        self.inner.shutdown(timeout)
    }
}

#[derive(Debug, Clone)]
enum SettingsMutation {
    Replace(AppConfig),
    ToggleFloatButton,
}

trait SettingsBackend {
    fn load(&mut self) -> AppConfig;
    fn persist(&mut self, config: &AppConfig) -> Result<(), String>;
    fn set_telemetry(&mut self, enabled: bool);
    fn set_minidumps(&mut self, enabled: bool);
    fn set_float_button(&mut self, visible: bool);
    fn publish(&mut self, config: &AppConfig);
}

struct AppBackend<'a> {
    app: &'a AppHandle,
}

impl SettingsBackend for AppBackend<'_> {
    fn load(&mut self) -> AppConfig {
        AppConfig::load()
    }

    fn persist(&mut self, config: &AppConfig) -> Result<(), String> {
        config.save().map_err(|err| format!("{err}"))
    }

    fn set_telemetry(&mut self, enabled: bool) {
        apply_telemetry(enabled);
    }

    fn set_minidumps(&mut self, enabled: bool) {
        apply_minidumps(enabled);
    }

    fn set_float_button(&mut self, visible: bool) {
        crate::ensure_float_window(self.app, visible);
    }

    fn publish(&mut self, config: &AppConfig) {
        let _ = self.app.emit(EVENT_CONFIG, config);
    }
}

pub(crate) fn replace(app: &AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    apply_to_app(app, SettingsMutation::Replace(config))
}

pub(crate) fn toggle_float_button(app: &AppHandle) -> Result<AppConfig, String> {
    apply_to_app(app, SettingsMutation::ToggleFloatButton)
}

fn apply_to_app(app: &AppHandle, mutation: SettingsMutation) -> Result<AppConfig, String> {
    let _guard = MUTATION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    apply_mutation(&mut AppBackend { app }, mutation)
}

fn apply_mutation(
    backend: &mut impl SettingsBackend,
    mutation: SettingsMutation,
) -> Result<AppConfig, String> {
    let config = match mutation {
        SettingsMutation::Replace(config) => {
            validate(&config)?;
            config
        }
        SettingsMutation::ToggleFloatButton => {
            let mut config = backend.load();
            config.general.float_button = !config.general.float_button;
            config
        }
    };

    backend.persist(&config)?;

    let reporting = sentry_client_enabled(&config);
    backend.set_telemetry(reporting);
    backend.set_minidumps(reporting);
    backend.set_float_button(config.general.float_button);
    backend.publish(&config);
    Ok(config)
}

fn validate(config: &AppConfig) -> Result<(), String> {
    if !matches!(config.general.theme.as_str(), "system" | "dark" | "light") {
        return Err(format!("invalid theme: {}", config.general.theme));
    }
    if config.general.typing_wpm == 0 {
        return Err("typing speed baseline must be greater than zero".into());
    }
    if config.cleanup.timeout_secs == 0 {
        return Err("cleanup timeout must be greater than zero".into());
    }
    Ok(())
}

pub(crate) fn reporting_enabled(config: &AppConfig) -> bool {
    config.general.crash_reports && !config.general.local_only
}

pub(crate) fn sentry_client_enabled(config: &AppConfig) -> bool {
    reporting_enabled(config)
        && (!cfg!(debug_assertions)
            || std::env::var("PICKSCRIBE_SENTRY_DEBUG").ok().as_deref() == Some("1"))
}

pub(crate) fn telemetry_enabled() -> bool {
    TELEMETRY_ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn initialize_reporting(enabled: bool, client: Option<Arc<sentry::Client>>) {
    if let Some(client) = client {
        let _ = SENTRY_CLIENT.set(client);
    }
    apply_telemetry(enabled);
    apply_minidumps(enabled);
}

pub(crate) fn transport_factory() -> Arc<dyn TransportFactory> {
    let factory = sentry::transports::DefaultTransportFactory;
    Arc::new(move |options: &ClientOptions| {
        Arc::new(GatedTransport {
            inner: factory.create_transport(options),
        }) as Arc<dyn Transport>
    })
}

fn apply_telemetry(enabled: bool) {
    TELEMETRY_ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        if let Some(client) = SENTRY_CLIENT.get() {
            sentry::Hub::main().bind_client(Some(Arc::clone(client)));
        }
    } else {
        sentry::Hub::main().bind_client(None);
    }
}

fn apply_minidumps(enabled: bool) {
    let mut guard = MINIDUMP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !enabled {
        guard.take();
        return;
    }
    if guard.is_some() {
        return;
    }
    let Some(client) = SENTRY_CLIENT.get() else {
        return;
    };
    match tauri_plugin_sentry::minidump::init(client) {
        Ok(handle) => *guard = Some(handle),
        Err(err) => eprintln!("failed to initialize Sentry minidump handler: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[derive(Debug, Clone, Copy)]
    enum MutationPath {
        FullSave,
        FloatToggle,
    }

    struct RecordingBackend {
        stored: AppConfig,
        fail_persist: bool,
        calls: Vec<String>,
        published: usize,
    }

    impl RecordingBackend {
        fn new(fail_persist: bool) -> Self {
            Self {
                stored: AppConfig::default(),
                fail_persist,
                calls: Vec::new(),
                published: 0,
            }
        }
    }

    impl SettingsBackend for RecordingBackend {
        fn load(&mut self) -> AppConfig {
            self.calls.push("load".into());
            self.stored.clone()
        }

        fn persist(&mut self, config: &AppConfig) -> Result<(), String> {
            self.calls.push("persist".into());
            if self.fail_persist {
                return Err("disk full".into());
            }
            self.stored = config.clone();
            Ok(())
        }

        fn set_telemetry(&mut self, enabled: bool) {
            self.calls.push(format!("telemetry:{enabled}"));
        }

        fn set_minidumps(&mut self, enabled: bool) {
            self.calls.push(format!("minidumps:{enabled}"));
        }

        fn set_float_button(&mut self, visible: bool) {
            self.calls.push(format!("float:{visible}"));
        }

        fn publish(&mut self, _config: &AppConfig) {
            self.calls.push("publish".into());
            self.published += 1;
        }
    }

    fn mutation_for(path: MutationPath) -> SettingsMutation {
        match path {
            MutationPath::FullSave => {
                let mut config = AppConfig::default();
                config.general.float_button = false;
                SettingsMutation::Replace(config)
            }
            MutationPath::FloatToggle => SettingsMutation::ToggleFloatButton,
        }
    }

    #[test]
    fn every_mutation_path_persists_before_runtime_effects_and_publication() {
        for path in [MutationPath::FullSave, MutationPath::FloatToggle] {
            let mut backend = RecordingBackend::new(false);

            let result = apply_mutation(&mut backend, mutation_for(path)).unwrap();
            let reporting = sentry_client_enabled(&result);

            assert!(!result.general.float_button, "path: {path:?}");
            let expected = match path {
                MutationPath::FullSave => vec![
                    "persist".into(),
                    format!("telemetry:{reporting}"),
                    format!("minidumps:{reporting}"),
                    "float:false".into(),
                    "publish".into(),
                ],
                MutationPath::FloatToggle => vec![
                    "load".into(),
                    "persist".into(),
                    format!("telemetry:{reporting}"),
                    format!("minidumps:{reporting}"),
                    "float:false".into(),
                    "publish".into(),
                ],
            };
            assert_eq!(backend.calls, expected, "path: {path:?}");
            assert_eq!(backend.published, 1, "path: {path:?}");
        }
    }

    #[test]
    fn persistence_failure_has_no_runtime_effect_or_publication_for_any_mutation_path() {
        for path in [MutationPath::FullSave, MutationPath::FloatToggle] {
            let mut backend = RecordingBackend::new(true);

            let error = apply_mutation(&mut backend, mutation_for(path)).unwrap_err();

            assert_eq!(error, "disk full", "path: {path:?}");
            let expected = match path {
                MutationPath::FullSave => vec!["persist"],
                MutationPath::FloatToggle => vec!["load", "persist"],
            };
            assert_eq!(backend.calls, expected, "path: {path:?}");
            assert_eq!(backend.published, 0, "path: {path:?}");
            assert!(backend.stored.general.float_button, "path: {path:?}");
        }
    }

    #[test]
    fn invalid_full_save_has_no_persistence_runtime_effect_or_publication() {
        let mut backend = RecordingBackend::new(false);
        let mut config = AppConfig::default();
        config.general.theme = "sepia".into();

        let error = apply_mutation(&mut backend, SettingsMutation::Replace(config)).unwrap_err();

        assert_eq!(error, "invalid theme: sepia");
        assert!(backend.calls.is_empty());
        assert_eq!(backend.published, 0);
    }

    #[test]
    fn float_toggle_is_not_blocked_by_unrelated_persisted_values() {
        let mut backend = RecordingBackend::new(false);
        backend.stored.general.theme = "sepia".into();

        let result = apply_mutation(&mut backend, SettingsMutation::ToggleFloatButton).unwrap();

        assert!(!result.general.float_button);
        assert_eq!(backend.published, 1);
    }

    #[test]
    fn local_only_mode_disables_reporting_even_when_crash_reports_are_persisted() {
        let mut backend = RecordingBackend::new(false);
        let mut config = AppConfig::default();
        config.general.local_only = true;
        config.general.crash_reports = true;

        apply_mutation(&mut backend, SettingsMutation::Replace(config)).unwrap();

        assert_eq!(
            backend.calls,
            vec![
                "persist",
                "telemetry:false",
                "minidumps:false",
                "float:true",
                "publish",
            ]
        );
    }

    struct CountingTransport(AtomicUsize);

    impl Transport for CountingTransport {
        fn send_envelope(&self, _envelope: Envelope) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn transport_drops_all_envelopes_while_reporting_is_disabled() {
        let inner = Arc::new(CountingTransport(AtomicUsize::new(0)));
        let transport = GatedTransport {
            inner: Arc::clone(&inner) as Arc<dyn Transport>,
        };
        TELEMETRY_ENABLED.store(false, Ordering::Relaxed);

        transport.send_envelope(Envelope::new());
        assert_eq!(inner.0.load(Ordering::Relaxed), 0);

        TELEMETRY_ENABLED.store(true, Ordering::Relaxed);
        transport.send_envelope(Envelope::new());
        assert_eq!(inner.0.load(Ordering::Relaxed), 1);
        TELEMETRY_ENABLED.store(false, Ordering::Relaxed);
    }
}
