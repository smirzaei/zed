use x11rb::protocol::xproto::KeyPressEvent;

use super::X11Client;

/// Where a key on X11 goes while an input method is connected.
#[derive(Debug, PartialEq, Eq)]
enum KeyRoute {
    /// The input method sees the key. GPUI sees it only if the server forwards it back.
    InputMethod,
    /// GPUI handles the key as it does without an input method. `end_composition` says the
    /// input method may hold a composition, which GPUI ends first.
    Direct { end_composition: bool },
}

impl KeyRoute {
    /// A composing input method takes printable keys, so a key that the focused input takes no
    /// text from must bypass it.
    fn new(accepts_text: bool, composing: bool, forwarded_since_reset: bool) -> Self {
        if accepts_text {
            return Self::InputMethod;
        }
        Self::Direct {
            end_composition: composing || forwarded_since_reset,
        }
    }
}

impl X11Client {
    /// Returns whether a key bypasses the input method. A bypassing key first ends any
    /// composition the input method may hold, as focus-out does. A key for an unknown window
    /// bypasses it and ends nothing.
    pub(super) fn bypass_xim(&self, key: &KeyPressEvent) -> bool {
        let Some(window) = self.get_window(key.event) else {
            return true;
        };
        let accepts_text = window.query_accepts_focused_text_input();
        let route = {
            let state = self.0.borrow();
            KeyRoute::new(accepts_text, state.composing, state.forwarded_since_reset)
        };
        let KeyRoute::Direct { end_composition } = route else {
            return false;
        };
        if end_composition {
            self.reset_ime();
            window.handle_ime_delete();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::KeyRoute;

    #[test]
    fn keys_reach_the_input_method_only_while_text_is_accepted() {
        assert_eq!(KeyRoute::new(true, false, false), KeyRoute::InputMethod);
        assert_eq!(
            KeyRoute::new(false, false, true),
            KeyRoute::Direct {
                end_composition: true
            }
        );
        assert_eq!(
            KeyRoute::new(false, false, false),
            KeyRoute::Direct {
                end_composition: false
            }
        );
        assert_eq!(
            KeyRoute::new(false, true, false),
            KeyRoute::Direct {
                end_composition: true
            }
        );
        assert_eq!(KeyRoute::new(true, true, true), KeyRoute::InputMethod);
    }
}
