use std::borrow::Cow;

use anyhow::{Context as _, Result, ensure};

use crate::{
    AtlasKey, AtlasTextureKind, Bounds, DevicePixels, Hsla, MonochromeSprite, Point,
    PolychromeSprite, Size, TransformationMatrix, Window,
};

/// Identifies a bitmap that an app rasterized itself, in a window's sprite atlas.
///
/// The `u64` is the app's choice. Two bitmaps that differ in any way, such as in the scale factor
/// they were rasterized at, need different ids. Ids of masks and of color bitmaps don't clash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppSpriteKey {
    /// A mask with one byte of coverage per pixel. It is painted in a color, as a monochrome
    /// sprite, so tests read it through `Window::painted_monochrome_sprites`.
    Mask(u64),
    /// A color bitmap with four bytes per pixel in blue, green, red, alpha order, where the color
    /// channels are not premultiplied by alpha.
    Color(u64),
}

impl AppSpriteKey {
    pub(crate) fn texture_kind(self) -> AtlasTextureKind {
        match self {
            Self::Mask(_) => AtlasTextureKind::Monochrome,
            Self::Color(_) => AtlasTextureKind::Polychrome,
        }
    }

    fn check_bitmap(self, size: Size<DevicePixels>, bytes: &[u8]) -> Result<()> {
        let bytes_per_pixel = match self {
            Self::Mask(_) => 1,
            Self::Color(_) => 4,
        };
        ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "the bitmap of {self:?} is empty: {size:?}"
        );
        let expected_len = (size.width.0 as usize)
            .checked_mul(size.height.0 as usize)
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel));
        ensure!(
            expected_len == Some(bytes.len()),
            "the bitmap of {self:?} is {size:?} but has {} bytes",
            bytes.len()
        );
        Ok(())
    }
}

impl Window {
    /// Paints a bitmap, which the app rasterized itself, into the scene for the next frame at the
    /// current z-index.
    ///
    /// `origin` is the bitmap's top-left corner in the window's device pixels, so the app decides
    /// how it snaps to the pixel grid. The atlas keeps the bitmap under `key`, and `rasterize`
    /// runs only when it isn't there. It returns the bitmap's size, which can't be empty, and its
    /// pixels row by row from the top, with no padding, in the layout `key` documents.
    ///
    /// A mask is painted in `color`. A color bitmap keeps its own colors and ignores it. Element
    /// opacity and the content mask apply as they do to glyphs. When `rasterize` fails or returns
    /// a bitmap that doesn't match its size, the error is returned and nothing is stored or
    /// painted.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_app_sprite(
        &mut self,
        origin: Point<DevicePixels>,
        key: AppSpriteKey,
        color: Hsla,
        rasterize: impl FnOnce() -> Result<(Size<DevicePixels>, Vec<u8>)>,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let mut rasterize = Some(rasterize);
        let tile = self
            .sprite_atlas
            .get_or_insert_with(AtlasKey::App(key), &mut || {
                let rasterize = rasterize
                    .take()
                    .context("the atlas built an app sprite twice")?;
                let (size, bytes) = rasterize()?;
                key.check_bitmap(size, &bytes)?;
                Ok(Some((size, Cow::Owned(bytes))))
            })?
            .expect("Callback above only errors or returns Some");
        let bounds = Bounds {
            origin: origin.map(Into::into),
            size: tile.bounds.size.map(Into::into),
        };
        let content_mask = self.snapped_content_mask();
        let opacity = self.element_opacity();

        match key {
            AppSpriteKey::Mask(_) => self.next_frame.scene.insert_primitive(MonochromeSprite {
                order: 0,
                pad: 0,
                bounds,
                content_mask,
                color: color.opacity(opacity),
                tile,
                transformation: TransformationMatrix::unit(),
            }),
            AppSpriteKey::Color(_) => self.next_frame.scene.insert_primitive(PolychromeSprite {
                order: 0,
                pad: 0,
                grayscale: false.into(),
                bounds,
                corner_radii: Default::default(),
                content_mask,
                tile,
                opacity,
            }),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use anyhow::{Context as _, Result};

    use crate::{
        AnyWindowHandle, AppContext as _, AppSpriteKey, AtlasKey, AtlasTextureKind, Bounds,
        ContentMask, Context, DevicePixels, Hsla, IntoElement, Point, Render, ScaledPixels, Size,
        Styled, TestAppContext, Window, canvas, hsla, point, px, size,
    };

    const RED: Hsla = hsla(0., 1., 0.5, 1.);

    struct Painter<F>(F);

    impl<F: Fn(&mut Window) + Clone + 'static> Render for Painter<F> {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let paint = self.0.clone();
            canvas(|_, _, _| {}, move |_, _, window, _| paint(window)).size_full()
        }
    }

    fn paint_window(
        cx: &mut TestAppContext,
        paint: impl Fn(&mut Window) + Clone + 'static,
    ) -> AnyWindowHandle {
        cx.add_window(|_, _| Painter(paint)).into()
    }

    fn redraw(cx: &mut TestAppContext, window: AnyWindowHandle) {
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
    }

    fn bitmap(width: i32, height: i32, bytes_per_pixel: usize) -> (Size<DevicePixels>, Vec<u8>) {
        let bytes = vec![0xff; width as usize * height as usize * bytes_per_pixel];
        (size(DevicePixels(width), DevicePixels(height)), bytes)
    }

    fn device_origin(x: i32, y: i32) -> Point<DevicePixels> {
        point(DevicePixels(x), DevicePixels(y))
    }

    fn scaled_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(width), ScaledPixels(height)),
        )
    }

    #[gpui::test]
    fn test_mask_paints_in_color_at_device_origin(cx: &mut TestAppContext) {
        let window = paint_window(cx, |window| {
            let (size, bytes) = bitmap(2, 3, 1);
            window
                .paint_app_sprite(device_origin(5, 7), AppSpriteKey::Mask(1), RED, move || {
                    Ok((size, bytes))
                })
                .unwrap();
        });

        cx.update_window(window, |_, window, _| {
            let sprites = window.painted_monochrome_sprites();
            assert_eq!(sprites.len(), 1);
            assert_eq!(sprites[0].bounds, scaled_bounds(5., 7., 2., 3.));
            assert_eq!(sprites[0].color, RED);
            assert_eq!(
                sprites[0].tile.texture_id.kind,
                AtlasTextureKind::Monochrome
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_color_sprite_paints_untinted_and_apart_from_a_mask_with_the_same_id(
        cx: &mut TestAppContext,
    ) {
        let window = paint_window(cx, |window| {
            window
                .paint_app_sprite(device_origin(0, 0), AppSpriteKey::Mask(1), RED, || {
                    Ok(bitmap(2, 2, 1))
                })
                .unwrap();
            window
                .paint_app_sprite(device_origin(10, 0), AppSpriteKey::Color(1), RED, || {
                    Ok(bitmap(3, 2, 4))
                })
                .unwrap();
        });

        cx.update_window(window, |_, window, _| {
            let masks = window.painted_monochrome_sprites();
            let colors = &window.rendered_frame.scene.polychrome_sprites;
            assert_eq!(masks.len(), 1);
            assert_eq!(colors.len(), 1);
            assert_eq!(masks[0].bounds, scaled_bounds(0., 0., 2., 2.));
            assert_eq!(colors[0].bounds, scaled_bounds(10., 0., 3., 2.));
            assert_eq!(colors[0].tile.texture_id.kind, AtlasTextureKind::Polychrome);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_sprite_rasterizes_once_across_redraws(cx: &mut TestAppContext) {
        let painted = Rc::new(Cell::new(0));
        let rasterized = Rc::new(Cell::new(0));
        let window = paint_window(cx, {
            let painted = painted.clone();
            let rasterized = rasterized.clone();
            move |window| {
                painted.set(painted.get() + 1);
                window
                    .paint_app_sprite(device_origin(0, 0), AppSpriteKey::Mask(1), RED, || {
                        rasterized.set(rasterized.get() + 1);
                        Ok(bitmap(2, 2, 1))
                    })
                    .unwrap();
            }
        });
        redraw(cx, window);
        assert_eq!(painted.get(), 2);
        assert_eq!(rasterized.get(), 1);

        cx.update_window(window, |_, window, _| {
            assert_eq!(window.painted_monochrome_sprites().len(), 1);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_opacity_and_content_mask_apply(cx: &mut TestAppContext) {
        let window = paint_window(cx, |window| {
            let mask = ContentMask {
                bounds: Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.))),
            };
            window.with_content_mask(Some(mask), |window| {
                window.with_element_opacity(Some(0.5), |window| {
                    window
                        .paint_app_sprite(device_origin(2, 2), AppSpriteKey::Mask(1), RED, || {
                            Ok(bitmap(2, 2, 1))
                        })
                        .unwrap();
                    window
                        .paint_app_sprite(device_origin(2, 2), AppSpriteKey::Color(1), RED, || {
                            Ok(bitmap(2, 2, 4))
                        })
                        .unwrap();
                    window
                        .paint_app_sprite(device_origin(50, 50), AppSpriteKey::Mask(1), RED, || {
                            Ok(bitmap(2, 2, 1))
                        })
                        .unwrap();
                });
            });
        });

        cx.update_window(window, |_, window, _| {
            let clip = Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.)))
                .scale(window.scale_factor());
            let masks = window.painted_monochrome_sprites();
            let colors = &window.rendered_frame.scene.polychrome_sprites;
            assert_eq!(masks.len(), 1, "the sprite outside the mask is culled");
            assert_eq!(colors.len(), 1);
            assert_eq!(masks[0].content_mask.bounds, clip);
            assert_eq!(colors[0].content_mask.bounds, clip);
            assert_eq!(masks[0].color, RED.opacity(0.5));
            assert_eq!(colors[0].opacity, 0.5);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_rejected_bitmaps_are_neither_stored_nor_painted(cx: &mut TestAppContext) {
        let next_bitmap = Rc::new(RefCell::new(None));
        let outcomes: Rc<RefCell<Vec<Result<()>>>> = Rc::default();
        let window = paint_window(cx, {
            let next_bitmap = next_bitmap.clone();
            let outcomes = outcomes.clone();
            move |window| {
                let outcome = window.paint_app_sprite(
                    device_origin(0, 0),
                    AppSpriteKey::Mask(1),
                    RED,
                    || {
                        next_bitmap
                            .borrow_mut()
                            .take()
                            .context("failed to rasterize")
                    },
                );
                outcomes.borrow_mut().push(outcome);
            }
        });

        let key = AtlasKey::App(AppSpriteKey::Mask(1));
        let rejected = [
            None,
            Some(bitmap(0, 0, 1)),
            Some(bitmap(2, 2, 4)),
            Some((size(DevicePixels(2), DevicePixels(2)), vec![0xff; 3])),
        ];
        for next in rejected {
            *next_bitmap.borrow_mut() = next;
            redraw(cx, window);
            assert!(outcomes.borrow_mut().pop().unwrap().is_err());
            cx.update_window(window, |_, window, _| {
                assert!(window.painted_monochrome_sprites().is_empty());
                assert!(!window.sprite_atlas.contains(&key));
            })
            .unwrap();
        }

        *next_bitmap.borrow_mut() = Some(bitmap(2, 2, 1));
        redraw(cx, window);
        assert!(outcomes.borrow_mut().pop().unwrap().is_ok());
        cx.update_window(window, |_, window, _| {
            assert_eq!(window.painted_monochrome_sprites().len(), 1);
            assert!(window.sprite_atlas.contains(&key));
        })
        .unwrap();
    }
}
