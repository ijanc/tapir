use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Format a duration as human-readable elapsed time.
///
/// - Under 60s: `"37s"`
/// - Under 1h: `"1m 37s"`
/// - 1h+: `"1h 2m 3s"`
fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

pub struct ThinkingTimer {
    stop: Arc<AtomicBool>,
    start: Instant,
    handle: Option<JoinHandle<()>>,
}

impl ThinkingTimer {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let start = Instant::now();

        let stop2 = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            eprint!("\n* pretending to thinking...");
            let _ = std::io::stderr().flush();

            while !stop2.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(1));
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let elapsed = start.elapsed();
                let fmt = format_duration(elapsed);
                eprint!("\r\x1b[K* pretending to thinking... {fmt}");
                let _ = std::io::stderr().flush();
            }
        });

        Self {
            stop,
            start,
            handle: Some(handle),
        }
    }

    pub fn stop(mut self) -> Duration {
        let elapsed = self.start.elapsed();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let fmt = format_duration(elapsed);
        eprintln!("\r\x1b[K* Thinking... {fmt}\n");
        elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_start_stop_returns_duration() {
        let timer = ThinkingTimer::start();
        std::thread::sleep(Duration::from_millis(50));
        let elapsed = timer.stop();
        assert!(elapsed.as_millis() >= 40);
        assert!(elapsed.as_millis() < 2000);
    }

    #[test]
    fn format_seconds_only() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn format_minutes_and_seconds() {
        assert_eq!(format_duration(Duration::from_secs(60)), "1m 0s");
        assert_eq!(format_duration(Duration::from_secs(97)), "1m 37s");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59m 59s");
    }

    #[test]
    fn format_hours() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h 0m 0s");
        assert_eq!(format_duration(Duration::from_secs(3723)), "1h 2m 3s");
    }
}
