use crate::Window;

impl Window {
    /// For testing: sets whether this window builds its accessibility tree every frame, even with
    /// no assistive technology connected, and redraws the window.
    /// [`crate::Application::new_inaccessible`] still wins.
    pub fn set_a11y_forced(&mut self, forced: bool) {
        self.a11y.forced = forced;
        self.refresh();
    }

    /// For testing: the most recent accessibility tree that a frame built, or `None` if no frame
    /// has built one. Frames build one only while assistive technology is connected or
    /// [`Self::set_a11y_forced`] is on.
    pub fn a11y_tree(&self) -> Option<&accesskit::TreeUpdate> {
        self.a11y.debug.last_tree_update.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AnyWindowHandle, AppContext as _, Context, InteractiveElement as _, IntoElement, Render,
        Role, StatefulInteractiveElement as _, TestAppContext, Window, div,
    };

    #[gpui::test]
    fn test_forced_a11y_tree_exposes_labeled_nodes(cx: &mut TestAppContext) {
        struct LabeledButton;

        impl Render for LabeledButton {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().id("go").role(Role::Button).aria_label("Go")
            }
        }

        let window: AnyWindowHandle = cx.add_window(|_, _| LabeledButton).into();
        cx.update_window(window, |_, window, _| assert!(window.a11y_tree().is_none()))
            .unwrap();

        cx.update_window(window, |_, window, _| window.set_a11y_forced(true))
            .unwrap();
        cx.update_window(window, |_, window, _| {
            let tree = window.a11y_tree().expect("the forced window builds a tree");
            assert!(
                tree.nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Go") && node.role() == Role::Button)
            );
        })
        .unwrap();
    }
}
