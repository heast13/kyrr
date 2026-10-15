//! Start at login from inside a Flatpak: the Background portal puts kyrr into
//! the session's autostart, the sandbox cannot reach the folder by itself.
//! Not every desktop has the portal (Hyprland and other tiling compositors
//! often do not), so the caller needs to tell "said no" from "not there".

use ashpd::desktop::background::Background;
use ashpd::desktop::ResponseError;
use ashpd::Error;
use futures_lite::future;

/// Why the portal did not answer with a yes or a no.
#[derive(Debug)]
pub enum Failure {
    /// The user closed the desktop's dialog.
    Cancelled,
    /// No portal, no backend for it, or the request failed.
    Unavailable(String),
}

/// Asks the portal to start kyrr (with `--background`) at login, or not to.
/// The desktop may ask the user first, so this can take a while. Returns
/// whether kyrr will start at login, as the portal says.
pub fn request_autostart(on: bool) -> Result<bool, Failure> {
    future::block_on(async {
        let reply = Background::request()
            .reason("Start kyrr when you sign in")
            .auto_start(on)
            .command(["kyrr", "--background"])
            .send()
            .await
            .map_err(failure)?
            .response()
            .map_err(failure)?;
        Ok(reply.auto_start())
    })
}

fn failure(e: Error) -> Failure {
    match e {
        Error::Response(ResponseError::Cancelled) => Failure::Cancelled,
        e => Failure::Unavailable(e.to_string()),
    }
}
