//! Human-readable reports for chained runtime errors.
//!
//! `anyhow` retains every context added while an error bubbles up, but ordinary
//! `{error}` formatting only displays the outermost message. This module renders
//! the complete cause chain and, when requested through the standard Rust
//! environment variables, the captured backtrace.

use anyhow::Error;
use std::backtrace::BacktraceStatus;
use std::fmt::Write as _;

/// Returns whether the process was asked to display library error backtraces.
///
/// `RUST_LIB_BACKTRACE` has priority, matching `anyhow`'s behaviour. If it is
/// absent, `RUST_BACKTRACE` is used as the fallback. Empty values and `0` disable
/// display; every other value enables it.
pub fn backtrace_requested() -> bool {
    std::env::var_os("RUST_LIB_BACKTRACE")
        .or_else(|| std::env::var_os("RUST_BACKTRACE"))
        .is_some_and(|value| !value.is_empty() && value.to_string_lossy() != "0")
}

/// Formats an error, all of its accumulated contexts, and an optional backtrace.
pub fn format_error(error: &Error) -> String {
    format_error_with_backtrace(error, backtrace_requested())
}

fn format_error_with_backtrace(error: &Error, show_backtrace: bool) -> String {
    let mut report = String::new();
    let mut chain = error.chain();

    if let Some(message) = chain.next() {
        let _ = writeln!(report, "Erreur: {message}");
    }
    for (index, cause) in chain.enumerate() {
        let _ = writeln!(report, "  {}. {cause}", index + 1);
    }

    if show_backtrace {
        match error.backtrace().status() {
            BacktraceStatus::Captured => {
                let _ = writeln!(report, "\nBacktrace:\n{}", error.backtrace());
            }
            BacktraceStatus::Disabled => {
                report.push_str(
                    "\nBacktrace demandé, mais non capturé. Vérifiez que l'exécutable utilise une version d'`anyhow` avec le support des backtraces.\n",
                );
            }
            BacktraceStatus::Unsupported => {
                report.push_str("\nLes backtraces ne sont pas prises en charge sur cette plateforme.\n");
            }
            _ => {}
        }
    }

    while report.ends_with('\n') {
        report.pop();
    }
    report
}

/// Prints a complete error report to standard error.
pub fn print_error(error: &Error) {
    eprintln!("{}", format_error(error));
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, Context};

    #[test]
    fn report_lists_every_context_in_order() {
        let error = Err::<(), _>(anyhow!("cause initiale"))
            .context("appel de fonction")
            .context("cellule de page")
            .unwrap_err();
        let report = format_error_with_backtrace(&error, false);

        assert!(report.contains("Erreur: cellule de page"));
        assert!(report.contains("1. appel de fonction"));
        assert!(report.contains("2. cause initiale"));
        assert!(!report.contains("Backtrace:"));
    }
}
