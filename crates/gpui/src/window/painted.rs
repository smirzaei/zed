use crate::{MonochromeSprite, PolychromeSprite, SubpixelSprite, Window};

impl Window {
    /// Returns the monochrome sprites in the most recently rendered frame's scene. Monochrome
    /// glyphs, SVG icons, and app sprites with a mask key paint as these; emoji, images, and
    /// subpixel text do not. Bounds are in scaled pixels and not clipped to the content mask;
    /// sprites that don't intersect it are culled at paint time and won't appear.
    pub fn painted_monochrome_sprites(&self) -> &[MonochromeSprite] {
        &self.rendered_frame.scene.monochrome_sprites
    }

    /// Returns the subpixel sprites in the most recently rendered frame's scene. Subpixel text and
    /// app sprites with a subpixel key paint as these. Bounds are in scaled pixels and not clipped
    /// to the content mask; sprites that don't intersect it are culled at paint time and won't
    /// appear.
    pub fn painted_subpixel_sprites(&self) -> &[SubpixelSprite] {
        &self.rendered_frame.scene.subpixel_sprites
    }

    /// Returns the polychrome sprites in the most recently rendered frame's scene. Emoji, images,
    /// and app sprites with a color key paint as these; monochrome and subpixel glyphs do not.
    /// Bounds are in scaled pixels and not clipped to the content mask; sprites that don't
    /// intersect it are culled at paint time and won't appear.
    pub fn painted_polychrome_sprites(&self) -> &[PolychromeSprite] {
        &self.rendered_frame.scene.polychrome_sprites
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AnyWindowHandle, AppContext as _, Bounds, Context, IntoElement, ParentElement as _, Render,
        Styled as _, TestAppContext, TransformationMatrix, Window, canvas, div, hsla, point, px,
        size,
    };

    #[gpui::test]
    fn test_painted_monochrome_sprites_expose_svg_icon(cx: &mut TestAppContext) {
        const ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#;

        struct Icon;

        impl Render for Icon {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().pl(px(10.)).pt(px(20.)).child(
                    canvas(
                        |_, _, _| {},
                        |bounds, _, window, cx| {
                            window
                                .paint_svg(
                                    bounds,
                                    "icon.svg".into(),
                                    Some(ICON),
                                    TransformationMatrix::unit(),
                                    hsla(0., 0., 0., 1.),
                                    cx,
                                )
                                .expect("the icon paints");
                        },
                    )
                    .size(px(16.)),
                )
            }
        }

        let window: AnyWindowHandle = cx.add_window(|_, _| Icon).into();
        cx.update_window(window, |_, window, _| {
            let sprites = window.painted_monochrome_sprites();
            assert_eq!(sprites.len(), 1);
            assert_eq!(
                sprites[0].bounds,
                Bounds::new(point(px(10.), px(20.)), size(px(16.), px(16.)))
                    .scale(window.scale_factor())
            );
        })
        .unwrap();
    }
}
