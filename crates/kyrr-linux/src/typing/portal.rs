//! The XDG RemoteDesktop portal, which GNOME and KDE offer instead of the
//! virtual keyboard protocol.

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, Session};
use ashpd::enumflags2::BitFlags;
use ashpd::{AppID, Error};
use futures_lite::future::block_on;

use super::{restore_token, set_restore_token, APP_ID};

/// A started session. It lives only while text is typed: GNOME shows a
/// "remote control" indicator in the top bar for as long as one is open.
/// The restore token opens the next one without asking; if the user revoked
/// the permission, the portal asks again.
pub struct Keyboard {
    remote: RemoteDesktop,
    session: Session<RemoteDesktop>,
}

impl Keyboard {
    /// None when there is no portal, or the user said no.
    pub fn open() -> Option<Keyboard> {
        block_on(Self::start()).ok()
    }

    async fn start() -> Result<Keyboard, Error> {
        // A host app has no identity of its own for the portal; without one
        // the permission could not be remembered. Not needed in Flatpak.
        if let Ok(id) = AppID::try_from(APP_ID) {
            let _ = ashpd::register_host_app(id).await;
        }
        let remote = RemoteDesktop::new().await?;
        let session = remote.create_session(Default::default()).await?;
        let token = restore_token();
        let options = SelectDevicesOptions::default()
            .set_devices(BitFlags::from(DeviceType::Keyboard))
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token.as_deref());
        remote.select_devices(&session, options).await?.response()?;
        let started = remote.start(&session, None, Default::default()).await?.response()?;
        if !started.devices().contains(DeviceType::Keyboard) {
            return Err(Error::NoResponse);
        }
        if let Some(new) = started.restore_token() {
            set_restore_token(Some(new.to_owned()));
        }
        Ok(Keyboard { remote, session })
    }

    pub fn type_keysyms(&mut self, keysyms: &[u32]) -> bool {
        block_on(self.press_all(keysyms)).is_ok()
    }

    /// Presses Shift+Insert. Sent as key codes, which are the same on every
    /// layout, unlike keysyms.
    pub fn press_paste(&mut self) -> bool {
        block_on(self.shift_insert()).is_ok()
    }

    /// Ends the session, and with it the desktop's indicator.
    pub fn close(self) {
        let _ = block_on(self.session.close());
    }

    /// Shift is let go again even when Insert failed.
    async fn shift_insert(&self) -> Result<(), Error> {
        // Linux input event codes (KEY_LEFTSHIFT, KEY_INSERT).
        const SHIFT: i32 = 42;
        const INSERT: i32 = 110;
        let key = |code, state| {
            self.remote
                .notify_keyboard_keycode(&self.session, code, state, Default::default())
        };
        key(SHIFT, KeyState::Pressed).await?;
        let insert = async {
            key(INSERT, KeyState::Pressed).await?;
            key(INSERT, KeyState::Released).await
        }
        .await;
        let shift = key(SHIFT, KeyState::Released).await;
        insert.and(shift)
    }

    /// Each call is a round trip to the portal, which paces the keys.
    async fn press_all(&self, keysyms: &[u32]) -> Result<(), Error> {
        for &keysym in keysyms {
            let keysym = keysym as i32;
            for state in [KeyState::Pressed, KeyState::Released] {
                self.remote
                    .notify_keyboard_keysym(&self.session, keysym, state, Default::default())
                    .await?;
            }
        }
        Ok(())
    }
}
