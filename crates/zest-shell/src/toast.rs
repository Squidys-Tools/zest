//! Toast notifications ("done" signal after background conversion).
//!
//! WinRT `ToastNotificationManager` with a `ToastText02` (title + body)
//! template. An unpackaged exe has no registered AppUserModelID, so the
//! process claims `Zest` as an explicit AUMID before showing — attribution
//! gets proper icon/name once the MSI writes the Start-menu shortcut that
//! owns it (see `wix/`). Until then the toast may appear under the bare
//! "Zest" name or land silently in the action center.

use anyhow::Result;

/// AppUserModelID the toasts attribute to.
const AUMID: &str = "Zest";

/// Show a toast. Best-effort by design — callers log failures rather than
/// treating a missed notification as a job failure.
pub fn notify(title: &str, body: &str) -> Result<()> {
    notify_impl(title, body)
}

#[cfg(windows)]
fn notify_impl(title: &str, body: &str) -> Result<()> {
    use anyhow::Context;
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    let aumid = HSTRING::from(AUMID);
    // Idempotent: claims the AUMID for this process so toasts can be
    // attributed at all. Failure only costs attribution, not the toast.
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&aumid);
    }

    let doc = XmlDocument::new().context("create XmlDocument")?;
    doc.LoadXml(&HSTRING::from(toast_xml(title, body)))
        .context("parse toast XML")?;
    let toast = ToastNotification::CreateToastNotification(&doc).context("create toast")?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&aumid)
        .or_else(|_| ToastNotificationManager::CreateToastNotifier())
        .context("create toast notifier")?;
    notifier.Show(&toast).context("show toast")?;
    Ok(())
}

#[cfg(not(windows))]
fn notify_impl(title: &str, body: &str) -> Result<()> {
    tracing::info!(title, body, "toast unsupported on this platform");
    Ok(())
}

/// `ToastText02`: bold first line over a wrapped body.
fn toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastText02\">\
         <text id=\"1\">{}</text><text id=\"2\">{}</text>\
         </binding></visual></toast>",
        escape_xml(title),
        escape_xml(body),
    )
}

/// File names and error text go into XML verbatim, so the five entities get
/// escaped or a name like `a&b.png` would corrupt the toast payload.
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_xml_handles_all_five_entities() {
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn toast_xml_embeds_escaped_title_and_body() {
        let xml = toast_xml("Zest: a&b.png", "saved as c.png");
        assert!(xml.contains("<text id=\"1\">Zest: a&amp;b.png</text>"));
        assert!(xml.contains("<text id=\"2\">saved as c.png</text>"));
        // Parse-shaped sanity: the doc has one binding and two text nodes.
        assert_eq!(xml.matches("<text").count(), 2);
    }

    /// Manual check: `cargo test -p zest-shell shows_a_real_toast -- --ignored`
    /// pops a real toast — visual confirmation the WinRT path works on this
    /// machine. Ignored so it never fires in a normal test run.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn shows_a_real_toast() {
        notify(
            "Zest test toast",
            "If you can read this, toast notifications work.",
        )
        .expect("toast should show");
    }
}
