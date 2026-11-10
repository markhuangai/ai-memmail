use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture<T>(operation: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
    capture_at_level(super::LogLevel::Info, operation)
}

pub(crate) fn capture_at_level<T>(
    level: super::LogLevel,
    operation: impl FnOnce() -> T,
) -> (T, Vec<serde_json::Value>) {
    // A persistent dispatcher prevents untraced test threads from caching disabled callsites.
    super::init_tracing(super::LogLevel::Error);
    let output = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_env_filter(super::tracing_filter(level))
        .with_writer(output.clone())
        .finish();
    let result = tracing::subscriber::with_default(subscriber, operation);
    let bytes = output.0.lock().unwrap().clone();
    let events = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    if let Ok(path) = std::env::var("AI_MEMMAIL_DIAGNOSTIC_LOG_PATH") {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
    (result, events)
}
