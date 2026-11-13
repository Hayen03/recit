//! Terminal-session setup and restoration for the Forge TUI.
//!
//! `TerminalSession` owns raw mode, alternate screen, and bracketed-paste state. Explicit
//! restoration is idempotent and reports errors; `Drop` makes a final best-effort restore.
use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

/// Ratatui terminal type used by the Forge application.
pub type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Guards the player terminal mode and restores it when dropped.
pub struct TerminalSession {
    pub terminal: AppTerminal,
    restored: bool,
}

impl TerminalSession {
    /// Enters raw mode, switches to the alternate screen, and enables bracketed paste.
    ///
    /// The returned guard records which terminal features were enabled and restores them on drop.
    ///
    /// # Errors
    ///
    /// Propagates terminal setup or Ratatui initialization failures.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, Hide)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }
    /// Restores the terminal once and marks this session as inactive.
    ///
    /// Calling `restore` again after a successful restoration is a no-op.
    ///
    /// # Errors
    ///
    /// Propagates failures while leaving raw mode or restoring the screen.
    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show
        )?;
        self.terminal.show_cursor()?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
