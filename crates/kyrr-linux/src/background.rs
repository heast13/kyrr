//! Start at login from inside a Flatpak: only the Background portal can put
//! kyrr into the session's autostart, the sandbox cannot reach the folder.

use ashpd::desktop::background::Background;
use futures_lite::future;

/// Asks the portal to start kyrr (with `--background`) at login, or not to.
/// The desktop may ask the user first, so this can take a while. Returns
/// whether kyrr will start at login.
pub fn request_autostart(on: bool) -> Result<bool, String> {
    future::block_on(async {
        let reply = Background::request()
            .reason("Start kyrr when you sign in")
            .auto_start(on)
            .command(["kyrr", "--background"])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|e| e.to_string())?;
        Ok(reply.auto_start())
    })
}
