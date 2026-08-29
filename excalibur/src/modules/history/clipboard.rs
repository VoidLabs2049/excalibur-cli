use arboard::Clipboard;
use color_eyre::Result;

/// Clipboard manager for copying commands
pub struct ClipboardManager {
    clipboard: Option<Clipboard>,
}

impl std::fmt::Debug for ClipboardManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardManager")
            .field("available", &self.clipboard.is_some())
            .finish()
    }
}

impl ClipboardManager {
    /// Create a new clipboard manager
    pub fn new() -> Self {
        // Try to initialize clipboard, but don't fail if it's not available
        let clipboard = Clipboard::new().ok();

        Self { clipboard }
    }

    /// Copy text to the clipboard
    pub fn copy(&mut self, text: &str) -> Result<()> {
        if let Some(ref mut clipboard) = self.clipboard {
            clipboard.set_text(text)?;
            Ok(())
        } else {
            Err(color_eyre::eyre::eyre!(
                "Clipboard not available on this system"
            ))
        }
    }
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}
